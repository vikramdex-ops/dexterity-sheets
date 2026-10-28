// Test-only: the clock and unreachable! are fine here.
#![allow(clippy::disallowed_methods, clippy::unreachable)]

use std::sync::Arc;
use std::time::Duration;

use gridcraft_engine::Session;
use gridcraft_engine::model::Workbook;
use serde_json::json;

use crate::patch::{self, CellData, DiffCache, WbPatch};
use crate::proto::{BookInfo, ClientMsg, ServerMsg};
use crate::sync::{self, SharedDoc};

fn wb(s: &Session) -> Arc<Workbook> {
    s.active().expect("doc").wb.clone()
}

/// Everything shared about two workbooks is equal: sheet list, cells, sheet and workbook fields.
fn assert_same(a: &Workbook, b: &Workbook) {
    let names = |w: &Workbook| w.sheets.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(a), names(b), "sheet lists differ");
    assert_eq!(patch::workbook_fields(a).expect("a"), patch::workbook_fields(b).expect("b"), "workbook fields differ");
    for (sa, sb) in a.sheets.iter().zip(&b.sheets) {
        assert_eq!(patch::sheet_fields(sa).expect("a"), patch::sheet_fields(sb).expect("b"), "fields of {} differ", sa.name);
        let cells = |w: &Workbook, s: &gridcraft_engine::model::Sheet| {
            s.cells
                .iter()
                .map(|(at, c)| {
                    (
                        at,
                        CellData {
                            v: c.value.clone(),
                            f: c.formula.as_ref().map(|f| crate::patch::FormulaData { text: f.text.clone(), array: f.array, dynamic: f.dynamic }),
                            s: (c.style != gridcraft_engine::model::StyleId::DEFAULT).then(|| w.styles.get(c.style).clone()),
                        },
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(cells(a, sa), cells(b, sb), "cells of {} differ", sa.name);
    }
}

#[test]
fn every_kind_of_edit_replays_on_a_replica() {
    let mut s = Session::new();
    s.new_workbook();
    let mut replica = (*wb(&s)).clone();
    let mut cache = DiffCache::default();
    let steps: Vec<(&str, serde_json::Value)> = vec![
        ("cell.set", json!({"cell": "A1", "input": "5"})),
        ("cell.set", json!({"cell": "B1", "input": "=A1*2"})),
        ("cell.set", json!({"cell": "A2", "input": "Line 100-P-2001"})),
        ("home.bold", json!({"range": "A1:B1", "on": true})),
        ("home.columnWidth", json!({"range": "A1", "width": 140})),
        ("home.mergeCenter", json!({"range": "C1:E1"})),
        ("home.insertRows", json!({"rows": "1:1"})),
        ("home.insertSheet", json!({"name": "MTO"})),
        ("cell.set", json!({"sheet": "MTO", "cell": "A1", "input": "=Sheet1!B2+1"})),
        ("sheet.rename", json!({"sheet": "MTO", "name": "Material"})),
        ("sheet.tabColor", json!({"sheet": "Material", "color": "#2266AA"})),
        ("edit.undo", json!({})),
        ("home.insertSheet", json!({"name": "Scratch"})),
        ("home.deleteSheet", json!({"sheet": "Scratch"})),
        ("cell.set", json!({"cell": "A2", "input": ""})),
    ];
    for (id, p) in steps {
        let before = wb(&s);
        s.execute(id, p.clone()).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
        let after = wb(&s);
        let d = patch::diff(&before, &after, &mut cache).expect("diff");
        // Through JSON, as on the wire.
        let d: WbPatch = serde_json::from_slice(&serde_json::to_vec(&d).expect("ser")).expect("de");
        patch::apply(&mut replica, &d).expect("apply");
        assert_same(&replica, &after);
    }
}

fn book() -> BookInfo {
    BookInfo {
        id: "b".into(),
        name: "Line list".into(),
        created: 0,
        created_by: "a".into(),
        modified: 0,
        modified_by: "a".into(),
        seq: 0,
        open_by: vec![],
        source: None,
    }
}

#[test]
fn concurrent_edits_converge_in_server_order() {
    let base = gridcraft_engine::io::save_bytes(&Workbook::new(), "x.xlsx").expect("xlsx");
    let (mut a, mut da) = SharedDoc::open(book(), 0, &base, &[], vec![], vec![], false).expect("a");
    let (mut b, mut db) = SharedDoc::open(book(), 0, &base, &[], vec![], vec![], false).expect("b");
    let mut sa = Session::new();
    sa.add_document(da.clone());
    let mut sb = Session::new();
    sb.add_document(db.clone());

    // Both edit at once: A writes A1 and C1, B writes A1 (conflict) and B2 = A1*10.
    sa.execute("cell.set", json!({"cell": "A1", "input": "1"})).expect("a1");
    sa.execute("cell.set", json!({"cell": "C1", "input": "from A"})).expect("c1");
    sb.execute("cell.set", json!({"cell": "A1", "input": "2"})).expect("b a1");
    sb.execute("cell.set", json!({"cell": "B2", "input": "=A1*10"})).expect("b2");
    da = sa.active().expect("a").clone();
    db = sb.active().expect("b").clone();
    let (ca, pa) = a.local_changes(&mut da).expect("diff").expect("a changed");
    let (cb, pb) = b.local_changes(&mut db).expect("diff").expect("b changed");

    // The server takes A's first, then B's; everyone hears both, in that order.
    a.ordered(&mut da, 1, "a", 0, Some(ca), &pa).expect("a own");
    b.ordered(&mut db, 1, "a", 0, None, &pa).expect("b hears a");
    a.ordered(&mut da, 2, "b", 0, None, &pb).expect("a hears b");
    b.ordered(&mut db, 2, "b", 0, Some(cb), &pb).expect("b own");

    assert!(!a.has_pending() && !b.has_pending());
    assert_same(&da.wb, &db.wb);
    let sheet = da.wb.sheet(0).expect("sheet");
    let v = |r, c| sheet.cell(gridcraft_engine::core::CellRef::new(r, c)).map(|c| c.value.clone());
    assert_eq!(v(0, 0), Some(gridcraft_engine::core::Value::Number(2.0)), "B's A1 came last and wins");
    assert_eq!(v(0, 2), Some(gridcraft_engine::core::Value::Text("from A".into())));
    assert_eq!(v(1, 1), Some(gridcraft_engine::core::Value::Number(20.0)));
    // Who changed what is known on both sides.
    assert_eq!(a.last_edit("Sheet1", gridcraft_engine::core::CellRef::new(0, 0)).map(|e| e.0.as_str()), Some("b"));
    assert_eq!(b.last_edit("Sheet1", gridcraft_engine::core::CellRef::new(0, 2)).map(|e| e.0.as_str()), Some("a"));
}

#[test]
fn undo_takes_back_only_your_own_edit() {
    let base = gridcraft_engine::io::save_bytes(&Workbook::new(), "x.xlsx").expect("xlsx");
    let (mut a, da) = SharedDoc::open(book(), 0, &base, &[], vec![], vec![], false).expect("a");
    let (mut b, db) = SharedDoc::open(book(), 0, &base, &[], vec![], vec![], false).expect("b");
    let mut sa = Session::new();
    sa.add_document(da);
    let mut sb = Session::new();
    sb.add_document(db);
    sa.execute("cell.set", json!({"cell": "A1", "input": "mine"})).expect("a");
    let (ca, pa) = a.local_changes(sa.active_mut().expect("a")).expect("d").expect("p");
    sb.execute("cell.set", json!({"cell": "A2", "input": "theirs"})).expect("b");
    let (_, pb) = b.local_changes(sb.active_mut().expect("b")).expect("d").expect("p");
    a.ordered(sa.active_mut().expect("a"), 1, "a", 0, Some(ca), &pa).expect("own");
    a.ordered(sa.active_mut().expect("a"), 2, "b", 0, None, &pb).expect("theirs");
    sa.execute("edit.undo", json!({})).expect("undo");
    let sheet = sa.active().expect("a").wb.sheet(0).expect("s").clone();
    assert!(sheet.cell(gridcraft_engine::core::CellRef::new(0, 0)).is_none(), "own edit undone");
    assert_eq!(
        sheet.cell(gridcraft_engine::core::CellRef::new(1, 0)).map(|c| c.value.clone()),
        Some(gridcraft_engine::core::Value::Text("theirs".into())),
        "the other person's edit stays"
    );
}

fn recv(c: &crate::client::Client, want: impl Fn(&crate::client::Event) -> bool) -> crate::client::Event {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let e = c.events.recv_timeout(left).expect("timed out waiting for the server");
        if want(&e) {
            return e;
        }
    }
}

#[test]
fn two_people_edit_through_a_real_server() {
    let dir = std::env::temp_dir().join(format!("dexsheets-test-{}", crate::now_ms()));
    crate::server::set_user(&dir, "vikram", "Vikram", "pipe-123", crate::proto::Role::Admin).expect("user");
    crate::server::set_user(&dir, "anita", "Anita", "pipe-456", crate::proto::Role::Editor).expect("user");
    crate::server::set_user(&dir, "guest", "Guest", "view-789", crate::proto::Role::Viewer).expect("user");
    let (server, port) = crate::server::start_on(dir.clone(), "Test".into(), 0).expect("server");
    let cfg = |u: &str, p: &str| crate::client::Config {
        server: format!("127.0.0.1:{port}"),
        user: u.into(),
        password: p.into(),
        app: "test".into(),
        machine: "test".into(),
    };
    let wake: crate::client::Waker = Arc::new(|| {});

    // A wrong password is refused, and nothing is anonymous.
    let bad = crate::client::Client::connect(cfg("vikram", "nope!!"), wake.clone());
    assert!(matches!(recv(&bad, |_| true), crate::client::Event::Failed(_)));

    let v = crate::client::Client::connect(cfg("vikram", "pipe-123"), wake.clone());
    let a = crate::client::Client::connect(cfg("anita", "pipe-456"), wake.clone());
    let g = crate::client::Client::connect(cfg("guest", "view-789"), wake.clone());
    for c in [&v, &a, &g] {
        recv(c, |e| matches!(e, crate::client::Event::Connected { .. }));
    }
    v.send(ClientMsg::CreateBook { name: "Line List".into(), xlsx: None, source: None, sha256: None });
    let opened = |c: &crate::client::Client| match recv(c, |e| matches!(e, crate::client::Event::Message(ServerMsg::Opened { .. }))) {
        crate::client::Event::Message(ServerMsg::Opened { book, snapshot_seq, xlsx, log, last_edits, peers, .. }) => {
            (book, snapshot_seq, crate::proto::unb64(&xlsx).expect("b64"), log, last_edits, peers)
        }
        _ => unreachable!(),
    };
    let (bv, sv, xv, lv, ev, pv) = opened(&v);
    let id = bv.id.clone();
    a.send(ClientMsg::OpenBook { book: id.clone() });
    g.send(ClientMsg::OpenBook { book: id.clone() });
    let (ba, sa_, xa, la, ea, pa) = opened(&a);
    let (mut shv, mut dv) = SharedDoc::open(bv, sv, &xv, &lv, ev, pv, false).expect("v");
    let (mut sha, mut da) = SharedDoc::open(ba, sa_, &xa, &la, ea, pa, false).expect("a");
    let _ = opened(&g);

    // Vikram types; Anita sees it, attributed to him.
    let mut s = Session::new();
    s.add_document(dv.clone());
    s.execute("cell.set", json!({"cell": "B3", "input": "=6*7"})).expect("set");
    dv = s.active().expect("v").clone();
    let (cseq, p) = shv.local_changes(&mut dv).expect("d").expect("p");
    v.send(ClientMsg::Patch { book: id.clone(), cseq, patch: p, base: None });
    for (c, sh, d, inst) in [(&v, &mut shv, &mut dv, v.instance.clone()), (&a, &mut sha, &mut da, a.instance.clone())] {
        match recv(c, |e| matches!(e, crate::client::Event::Message(ServerMsg::Patched { .. }))) {
            crate::client::Event::Message(ServerMsg::Patched { seq, time, user, origin, patch, .. }) => {
                sh.ordered(d, seq, &user, time, crate::proto::own_cseq(origin.as_deref(), &inst), &patch).expect("ordered");
            }
            _ => unreachable!(),
        }
    }
    let at = gridcraft_engine::core::CellRef::new(2, 1);
    assert_eq!(da.wb.sheet(0).and_then(|s| s.cell(at)).map(|c| c.value.clone()), Some(gridcraft_engine::core::Value::Number(42.0)));
    assert_eq!(sha.last_edit("Sheet1", at).map(|e| e.0.as_str()), Some("vikram"));

    // A viewer's edit is refused by the server.
    g.send(ClientMsg::Patch { book: id.clone(), cseq: 1, patch: WbPatch::default(), base: None });
    assert!(matches!(
        recv(&g, |e| matches!(e, crate::client::Event::Message(ServerMsg::Rejected { .. }))),
        crate::client::Event::Message(ServerMsg::Rejected { .. })
    ));

    // The audit trail says who changed the cell.
    a.send(ClientMsg::History { book: id.clone(), sheet: Some("Sheet1".into()), cell: Some("B3".into()), user: None, limit: 10 });
    match recv(&a, |e| matches!(e, crate::client::Event::Message(ServerMsg::History { .. }))) {
        crate::client::Event::Message(ServerMsg::History { changes, .. }) => {
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0].user, "vikram");
            assert_eq!(changes[0].new, "=6*7");
        }
        _ => unreachable!(),
    }

    // A file shared from a folder keeps where it came from; sharing the same file again joins
    // it, and another file with the same name is shared beside it.
    let src = |path: &str| crate::proto::FileSource {
        path: path.into(),
        key: crate::proto::file_key("PC-1", path),
        user: "spoofed".into(),
        machine: "PC-1".into(),
    };
    v.send(ClientMsg::CreateBook { name: "Line List".into(), xlsx: None, source: Some(src(r"D:\Job 12\Line List.xlsx")), sha256: None });
    let (linked, ..) = opened(&v);
    assert_eq!(linked.name, "Line List (2)", "the name is free for a file from another folder");
    let source = linked.source.clone().expect("source");
    assert_eq!(source.user, "vikram", "the server records who shared it, whatever the client says");
    assert_eq!(source.key, r"pc-1|d:\job 12\line list.xlsx");
    a.send(ClientMsg::CreateBook { name: "Line List".into(), xlsx: None, source: Some(src(r"D:\JOB 12\line list.XLSX")), sha256: None });
    let (joined, ..) = opened(&a);
    assert_eq!(joined.id, linked.id, "the same file is joined, not shared twice");
    assert_eq!(crate::proto::file_key("PC-2", r"\\?\UNC\Files\Projects\a.xlsx"), r"\\files\projects\a.xlsx");
    assert_eq!(crate::proto::file_key("PC-2", r"\\?\D:\a.xlsx"), r"pc-2|d:\a.xlsx");

    // Stopping the sharing tells everyone who has it open.
    v.send(ClientMsg::DeleteBook { book: linked.id.clone(), stopped: true });
    match recv(&a, |e| matches!(e, crate::client::Event::Message(ServerMsg::BookGone { .. }))) {
        crate::client::Event::Message(ServerMsg::BookGone { book, by, stopped }) => {
            assert_eq!(book, linked.id);
            assert!(stopped);
            assert_eq!(by.as_deref(), Some("Vikram"));
        }
        _ => unreachable!(),
    }

    // A restarted server has every edit (log replay).
    drop(server);
    let again = crate::server::Server::open(dir.clone(), "Test".into()).expect("reopen");
    drop(again);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_app_installers_reach_office_pcs() {
    use crate::server::is_app_installer;
    assert!(is_app_installer("dexterity-sheets-1.2.0-windows-x64.msi"));
    assert!(is_app_installer("DexteritySheets-1.2.0-x64.MSI"));
    assert!(!is_app_installer("DexSheetsServer-1.2.0-x64.msi"), "the server's own installer is never offered");
    assert!(!is_app_installer("DexteritySheets-Setup.exe"));
    assert!(!is_app_installer("dexterity-sheets-1.2.0-windows-x64.msi.part"));
}

// ---------------------------------------------------------------------------------------------
// Persistence, sharing and collaboration regressions (the blank-shared-workbook report).
//
// Each test drives a real server on a throw-away data folder through the protocol, and checks
// the workbook *content* (cells, formulas, sheets, formatting) at every hop, including the
// server's own files on disk, rather than the fact that a workbook of that name exists.

type Client = crate::client::Client;

struct Lab {
    dir: std::path::PathBuf,
    port: u16,
    _server: Arc<crate::server::Server>,
}

impl Lab {
    fn new(tag: &str) -> Lab {
        let dir = std::env::temp_dir().join(format!("dexsheets-{tag}-{}-{}", std::process::id(), crate::now_ms()));
        crate::server::set_user(&dir, "usera", "User A", "pass-a-111", crate::proto::Role::Editor).expect("user");
        crate::server::set_user(&dir, "userb", "User B", "pass-b-222", crate::proto::Role::Editor).expect("user");
        crate::server::set_user(&dir, "viewer", "Viewer", "pass-v-333", crate::proto::Role::Viewer).expect("user");
        let (server, port) = crate::server::start_on(dir.clone(), "Test".into(), 0).expect("server");
        Lab { dir, port, _server: server }
    }

    /// A second server process on the same data folder, as after a crash: the first one never
    /// flushed or snapshotted, so only what it wrote before acknowledging each edit counts.
    fn restarted(&self) -> Lab {
        let (server, port) = crate::server::start_on(self.dir.clone(), "Test".into(), 0).expect("server again");
        Lab { dir: self.dir.clone(), port, _server: server }
    }

    fn connect(&self, user: &str, password: &str) -> Client {
        let cfg = crate::client::Config {
            server: format!("127.0.0.1:{}", self.port),
            user: user.into(),
            password: password.into(),
            app: "test".into(),
            machine: "test".into(),
        };
        let c = Client::connect(cfg, Arc::new(|| {}));
        recv(&c, |e| matches!(e, crate::client::Event::Connected { .. }));
        c
    }

    /// The workbook as the server's files hold it: newest snapshot plus the log after it.
    fn on_disk(&self, book: &str) -> Workbook {
        let dir = self.dir.join("books").join(book);
        let mut snaps: Vec<(u64, std::path::PathBuf)> = std::fs::read_dir(&dir)
            .expect("book folder")
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                Some((n.strip_prefix("snapshot-")?.strip_suffix(".xlsx")?.parse().ok()?, e.path()))
            })
            .collect();
        snaps.sort_by_key(|s| std::cmp::Reverse(s.0));
        let (snap_seq, path) = snaps.first().cloned().expect("a snapshot");
        let bytes = std::fs::read(path).expect("snapshot bytes");
        let (mut wb, _) = gridcraft_engine::io::open_bytes("book.xlsx", &bytes).expect("snapshot opens");
        let log = std::fs::read_to_string(dir.join("log.jsonl")).unwrap_or_default();
        for line in log.lines() {
            let e: crate::proto::Logged = serde_json::from_str(line).expect("log line");
            if e.seq > snap_seq {
                patch::apply(&mut wb, &e.patch).expect("replay");
            }
        }
        wb
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// One person's open copy of a shared workbook, edited through real engine commands.
struct Replica {
    sd: SharedDoc,
    s: Session,
    instance: String,
}

impl Replica {
    fn opened(c: &Client) -> Replica {
        match recv(c, |e| matches!(e, crate::client::Event::Message(ServerMsg::Opened { .. }))) {
            crate::client::Event::Message(ServerMsg::Opened { book, snapshot_seq, xlsx, log, last_edits, peers, sha256 }) => {
                assert!(!sha256.is_empty(), "the server sends the snapshot's checksum");
                let bytes = crate::proto::unb64_checked(&xlsx, &sha256).expect("intact download");
                let (sd, doc) = SharedDoc::open(book, snapshot_seq, &bytes, &log, last_edits, peers, false).expect("opens");
                let mut s = Session::new();
                s.add_document(doc);
                Replica { sd, s, instance: c.instance.clone() }
            }
            _ => unreachable!(),
        }
    }

    fn wb(&self) -> Arc<Workbook> {
        wb(&self.s)
    }

    fn text(&self, sheet: &str, a1: &str) -> String {
        let w = self.wb();
        let at = gridcraft_engine::core::CellRef::parse(a1).expect("a1");
        w.sheets.iter().find(|s| s.name == sheet).and_then(|s| s.cell(at)).map(|c| shown(&c.value)).unwrap_or_default()
    }

    /// Runs a command and returns the edit to send, as the window does after every change.
    fn edit(&mut self, id: &str, p: serde_json::Value) -> ClientMsg {
        self.s.execute(id, p.clone()).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
        let doc = self.s.active_mut().expect("doc");
        let (cseq, patch) = self.sd.local_changes(doc).expect("diff").expect("a change");
        ClientMsg::Patch { book: self.sd.book.id.clone(), cseq, patch, base: self.sd.base_of(cseq) }
    }

    /// Takes in ordered edits until this copy is at `seq`; returns the conflicts reported.
    fn catch_up(&mut self, c: &Client, seq: u64) -> Vec<crate::proto::Conflict> {
        let mut all = Vec::new();
        while self.sd.seq < seq {
            match recv(c, |e| matches!(e, crate::client::Event::Message(ServerMsg::Patched { .. }))) {
                crate::client::Event::Message(ServerMsg::Patched { seq, time, user, origin, patch, conflicts, .. }) => {
                    let own = crate::proto::own_cseq(origin.as_deref(), &self.instance);
                    let doc = self.s.active_mut().expect("doc");
                    let fx = self.sd.ordered(doc, seq, &user, time, own, &patch).expect("ordered");
                    assert!(fx.outgoing.is_none(), "no edits are in flight in these tests");
                    all.extend(conflicts);
                }
                _ => unreachable!(),
            }
        }
        all
    }
}

/// A cell's value as text, numbers without a trailing ".0".
fn shown(v: &gridcraft_engine::core::Value) -> String {
    match v {
        gridcraft_engine::core::Value::Number(n) => format!("{n}"),
        other => patch::value_text(other),
    }
}

/// A workbook with something of everything: text, numbers, formulas, two sheets, formatting.
fn rich_workbook() -> Arc<Workbook> {
    let mut s = Session::new();
    s.new_workbook();
    for (id, p) in [
        ("cell.set", json!({"cell": "A1", "input": "Distinctive text 7Q-Persist"})),
        ("cell.set", json!({"cell": "B1", "input": "42.5"})),
        ("cell.set", json!({"cell": "C1", "input": "=B1*2"})),
        ("home.bold", json!({"range": "A1", "on": true})),
        ("home.columnWidth", json!({"range": "A1", "width": 180})),
        ("home.insertSheet", json!({"name": "MTO"})),
        ("cell.set", json!({"sheet": "MTO", "cell": "A1", "input": "=Sheet1!C1+1"})),
        ("cell.set", json!({"sheet": "MTO", "cell": "B2", "input": "150-P-3001"})),
    ] {
        s.execute(id, p.clone()).unwrap_or_else(|e| panic!("{id} {p}: {e}"));
    }
    wb(&s)
}

fn share(c: &Client, name: &str, w: &Workbook) {
    let bytes = gridcraft_engine::io::save_bytes(w, "share.xlsx").expect("save");
    let sha256 = Some(crate::proto::sha256_hex(&bytes));
    c.send(ClientMsg::CreateBook { name: name.into(), xlsx: Some(crate::proto::b64(&bytes)), source: None, sha256 });
}

/// Tests A and B: a populated workbook, shared, is on the server's disk with its content and
/// opens with that content for someone else, formatting and second sheet included.
#[test]
fn a_shared_workbook_reaches_the_server_and_the_recipient_with_its_content() {
    let lab = Lab::new("share-content");
    let a = lab.connect("usera", "pass-a-111");
    let b = lab.connect("userb", "pass-b-222");
    let original = rich_workbook();
    share(&a, "Project Tracker", &original);
    let ra = Replica::opened(&a);
    assert!(sync::content_differences(&original, &ra.wb(), 10).is_empty(), "the sharer gets back what was uploaded");

    // The authoritative copy: the server's own files, read back from disk.
    let disk = lab.on_disk(&ra.sd.book.id);
    let missing = sync::content_differences(&original, &disk, 10);
    assert!(missing.is_empty(), "the server's disk lacks {missing:?}");
    assert!(!disk.sheets.iter().all(|s| s.cells.is_empty()), "never an empty workbook on the server");

    b.send(ClientMsg::ListBooks);
    match recv(&b, |e| matches!(e, crate::client::Event::Message(ServerMsg::Books { .. }))) {
        crate::client::Event::Message(ServerMsg::Books { books }) => assert!(books.iter().any(|x| x.id == ra.sd.book.id)),
        _ => unreachable!(),
    }
    b.send(ClientMsg::OpenBook { book: ra.sd.book.id.clone() });
    let rb = Replica::opened(&b);
    assert_eq!(rb.text("Sheet1", "A1"), "Distinctive text 7Q-Persist");
    assert_eq!(rb.text("Sheet1", "C1"), "85", "the formula's result");
    assert_eq!(rb.text("MTO", "A1"), "86", "a formula on the second sheet, across sheets");
    assert_same(&original, &rb.wb());
}

/// A damaged upload is refused, never stored as the workbook.
#[test]
fn a_damaged_upload_is_refused() {
    let lab = Lab::new("damaged");
    let a = lab.connect("usera", "pass-a-111");
    let bytes = gridcraft_engine::io::save_bytes(&rich_workbook(), "share.xlsx").expect("save");
    a.send(ClientMsg::CreateBook {
        name: "Damaged".into(),
        xlsx: Some(crate::proto::b64(&bytes)),
        source: None,
        sha256: Some(crate::proto::sha256_hex(b"something else")),
    });
    match recv(&a, |e| matches!(e, crate::client::Event::Message(ServerMsg::Error { .. } | ServerMsg::Opened { .. }))) {
        crate::client::Event::Message(ServerMsg::Error { message }) => assert!(message.contains("damaged"), "{message}"),
        _ => panic!("a damaged upload must not become a shared workbook"),
    }
    assert!(!lab.dir.join("books").read_dir().is_ok_and(|mut d| d.next().is_some()), "nothing was stored");
    // And a damaged download is never opened as a (blank) workbook.
    assert!(crate::proto::unb64_checked(&crate::proto::b64(&bytes), &crate::proto::sha256_hex(b"x")).is_err());
    assert!(SharedDoc::open(book(), 0, b"Line No,Size\n", &[], Vec::new(), Vec::new(), false).is_err(), "text is not a workbook");
}

/// Tests C, D, E, F and G: edits both ways, concurrent edits to different cells and to the same
/// cell, a server crash and restart, and reopening afterwards.
#[test]
fn owner_and_collaborator_edits_propagate_converge_and_survive_a_restart() {
    let lab = Lab::new("collab");
    let a = lab.connect("usera", "pass-a-111");
    let b = lab.connect("userb", "pass-b-222");
    share(&a, "Register", &rich_workbook());
    let mut ra = Replica::opened(&a);
    let id = ra.sd.book.id.clone();
    b.send(ClientMsg::OpenBook { book: id.clone() });
    let mut rb = Replica::opened(&b);

    // C: the owner edits; the collaborator receives it without reopening.
    a.send(ra.edit("cell.set", json!({"sheet": "Sheet1", "cell": "A2", "input": "owner edit"})));
    ra.catch_up(&a, 1);
    rb.catch_up(&b, 1);
    assert_eq!(rb.text("Sheet1", "A2"), "owner edit");
    // D: the collaborator edits a formula; the owner receives it, recalculated.
    b.send(rb.edit("cell.set", json!({"sheet": "Sheet1", "cell": "B2", "input": "=B1+0.5"})));
    rb.catch_up(&b, 2);
    ra.catch_up(&a, 2);
    assert_eq!(ra.text("Sheet1", "B2"), "43");
    // A structural edit (a sheet added) reaches the other side too.
    a.send(ra.edit("home.insertSheet", json!({"name": "Notes"})));
    ra.catch_up(&a, 3);
    rb.catch_up(&b, 3);
    assert!(rb.wb().sheets.iter().any(|s| s.name == "Notes"));

    // E1: both edit different cells at the same time (neither has seen the other's edit).
    let ea = ra.edit("cell.set", json!({"sheet": "Sheet1", "cell": "D5", "input": "from A"}));
    let eb = rb.edit("cell.set", json!({"sheet": "Sheet1", "cell": "E5", "input": "from B"}));
    a.send(ea);
    b.send(eb);
    assert!(ra.catch_up(&a, 5).is_empty() && rb.catch_up(&b, 5).is_empty(), "different cells are not a conflict");
    for r in [&ra, &rb] {
        assert_eq!((r.text("Sheet1", "D5"), r.text("Sheet1", "E5")), ("from A".into(), "from B".into()));
    }
    // E2: both edit the same cell at the same time. Policy: the server's order decides (the
    // later edit wins), and the conflict is reported to both, naming the other person.
    let ea = ra.edit("cell.set", json!({"sheet": "Sheet1", "cell": "F5", "input": "A's value"}));
    let eb = rb.edit("cell.set", json!({"sheet": "Sheet1", "cell": "F5", "input": "B's value"}));
    a.send(ea);
    std::thread::sleep(Duration::from_millis(150));
    b.send(eb);
    let ca = ra.catch_up(&a, 7);
    let cb = rb.catch_up(&b, 7);
    assert_eq!(ca, cb, "everyone hears of the same conflict");
    assert_eq!(ca.len(), 1, "{ca:?}");
    assert_eq!((ca[0].user.as_str(), ca[0].cell.as_str(), ca[0].theirs.as_str(), ca[0].yours.as_str()), ("usera", "F5", "A's value", "B's value"));
    assert_eq!(ra.text("Sheet1", "F5"), "B's value");
    assert_eq!(rb.text("Sheet1", "F5"), "B's value", "both copies converge");
    assert_same(&ra.wb(), &rb.wb());
    // The replaced value is not lost: it is in the cell's history.
    a.send(ClientMsg::History { book: id.clone(), sheet: Some("Sheet1".into()), cell: Some("F5".into()), user: None, limit: 10 });
    match recv(&a, |e| matches!(e, crate::client::Event::Message(ServerMsg::History { .. }))) {
        crate::client::Event::Message(ServerMsg::History { changes, .. }) => {
            assert!(changes.iter().any(|c| c.new == "A's value" && c.user == "usera"), "{changes:?}");
        }
        _ => unreachable!(),
    }

    // G: the server dies without a clean shutdown; every acknowledged edit is on its disk.
    let expected = ra.wb();
    drop((a, b));
    let lab2 = lab.restarted();
    let missing = sync::content_differences(&expected, &lab2.on_disk(&id), 10);
    assert!(missing.is_empty(), "lost on restart: {missing:?}");
    // F: both reopen in new sessions and get the latest state, nothing acknowledged missing.
    let a2 = lab2.connect("usera", "pass-a-111");
    let b2 = lab2.connect("userb", "pass-b-222");
    a2.send(ClientMsg::OpenBook { book: id.clone() });
    b2.send(ClientMsg::OpenBook { book: id.clone() });
    let (ra2, rb2) = (Replica::opened(&a2), Replica::opened(&b2));
    for r in [&ra2, &rb2] {
        assert_eq!(r.sd.seq, 7);
        assert_same(&expected, &r.wb());
    }
    std::mem::forget(lab2); // the folder is removed by `lab`
}

/// Test H: a viewer's edit is refused by the server, and nothing is served before signing in.
#[test]
fn viewers_cannot_edit_and_strangers_cannot_read() {
    let lab = Lab::new("perm");
    let a = lab.connect("usera", "pass-a-111");
    share(&a, "Confidential", &rich_workbook());
    let ra = Replica::opened(&a);
    let id = ra.sd.book.id.clone();
    let v = lab.connect("viewer", "pass-v-333");
    v.send(ClientMsg::OpenBook { book: id.clone() });
    let mut rv = Replica::opened(&v);
    let edit = match rv.edit("cell.set", json!({"cell": "A1", "input": "viewer was here"})) {
        ClientMsg::Patch { book, cseq, patch, base } => ClientMsg::Patch { book, cseq, patch, base },
        _ => unreachable!(),
    };
    v.send(edit);
    assert!(matches!(
        recv(&v, |e| matches!(e, crate::client::Event::Message(ServerMsg::Rejected { .. } | ServerMsg::Patched { .. }))),
        crate::client::Event::Message(ServerMsg::Rejected { .. })
    ));
    let disk = lab.on_disk(&id);
    let at = gridcraft_engine::core::CellRef::parse("A1").expect("a1");
    assert_eq!(disk.sheet(0).and_then(|s| s.cell(at)).map(|c| patch::value_text(&c.value)).as_deref(), Some("Distinctive text 7Q-Persist"));

    // No sign-in: asking for the workbook straight away gets an error, never its content.
    let mut raw = std::net::TcpStream::connect(("127.0.0.1", lab.port)).expect("connect");
    raw.set_read_timeout(Some(Duration::from_secs(10))).expect("timeout");
    crate::proto::write_frame(&mut raw, &ClientMsg::OpenBook { book: id }).expect("send");
    match crate::proto::read_frame::<_, ServerMsg>(&mut raw) {
        Ok(ServerMsg::Error { .. }) | Err(_) => {}
        Ok(other) => panic!("served without signing in: {other:?}"),
    }
}

/// Tests 13 and 14: a large workbook shares intact, and a burst of quick edits arrives
/// complete and in order on the other side and on the server's disk.
#[test]
fn a_large_workbook_and_a_burst_of_edits_arrive_complete_and_in_order() {
    let lab = Lab::new("large");
    let a = lab.connect("usera", "pass-a-111");
    let b = lab.connect("userb", "pass-b-222");
    let mut s = Session::new();
    s.new_workbook();
    let rows: Vec<serde_json::Value> =
        (0..2000).map(|r| json!([format!("150-P-{r:04}"), r, r as f64 * 0.25, format!("Line {r} spool"), "Open"])).collect();
    s.execute("range.setValues", json!({"range": "A1", "values": rows})).expect("values");
    s.execute("cell.set", json!({"cell": "F1", "input": "=SUM(B1:B2000)"})).expect("sum");
    let big = wb(&s);
    share(&a, "Large register", &big);
    let mut ra = Replica::opened(&a);
    b.send(ClientMsg::OpenBook { book: ra.sd.book.id.clone() });
    let mut rb = Replica::opened(&b);
    assert!(sync::content_differences(&big, &rb.wb(), 5).is_empty(), "all 10 000 cells reach the recipient");
    assert_eq!(rb.text("Sheet1", "F1"), "1999000");

    // 60 edits as fast as they can be made, none waiting for the one before.
    for i in 0..60 {
        a.send(ra.edit("cell.set", json!({"sheet": "Sheet1", "cell": format!("G{}", i + 1), "input": format!("burst {i}")})));
    }
    // And the same cell rewritten quickly: the last value is the one everyone keeps.
    for i in 0..10 {
        a.send(ra.edit("cell.set", json!({"sheet": "Sheet1", "cell": "H1", "input": format!("{i}")})));
    }
    ra.catch_up(&a, 70);
    rb.catch_up(&b, 70);
    for r in [&ra, &rb] {
        for i in 0..60 {
            assert_eq!(r.text("Sheet1", &format!("G{}", i + 1)), format!("burst {i}"));
        }
        assert_eq!(r.text("Sheet1", "H1"), "9");
    }
    assert!(!ra.sd.has_pending(), "every edit acknowledged");
    assert!(sync::content_differences(&ra.wb(), &lab.on_disk(&ra.sd.book.id), 5).is_empty());
}
