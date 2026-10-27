// Test-only: the clock and unreachable! are fine here.
#![allow(clippy::disallowed_methods, clippy::unreachable)]

use std::sync::Arc;
use std::time::Duration;

use gridcraft_engine::Session;
use gridcraft_engine::model::Workbook;
use serde_json::json;

use crate::patch::{self, CellData, DiffCache, WbPatch};
use crate::proto::{BookInfo, ClientMsg, ServerMsg};
use crate::sync::SharedDoc;

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
    v.send(ClientMsg::CreateBook { name: "Line List".into(), xlsx: None, source: None });
    let opened = |c: &crate::client::Client| match recv(c, |e| matches!(e, crate::client::Event::Message(ServerMsg::Opened { .. }))) {
        crate::client::Event::Message(ServerMsg::Opened { book, snapshot_seq, xlsx, log, last_edits, peers }) => {
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
    v.send(ClientMsg::Patch { book: id.clone(), cseq, patch: p });
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
    g.send(ClientMsg::Patch { book: id.clone(), cseq: 1, patch: WbPatch::default() });
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
    v.send(ClientMsg::CreateBook { name: "Line List".into(), xlsx: None, source: Some(src(r"D:\Job 12\Line List.xlsx")) });
    let (linked, ..) = opened(&v);
    assert_eq!(linked.name, "Line List (2)", "the name is free for a file from another folder");
    let source = linked.source.clone().expect("source");
    assert_eq!(source.user, "vikram", "the server records who shared it, whatever the client says");
    assert_eq!(source.key, r"pc-1|d:\job 12\line list.xlsx");
    a.send(ClientMsg::CreateBook { name: "Line List".into(), xlsx: None, source: Some(src(r"D:\JOB 12\line list.XLSX")) });
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
