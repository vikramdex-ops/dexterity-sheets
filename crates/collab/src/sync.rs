//! Keeping one open workbook in step with the server.
//!
//! The client keeps `confirmed`, the workbook as of the last edit the server ordered, and the
//! patches it sent that the server hasn't ordered yet (`pending`). What the person sees is always
//! `confirmed` plus `pending`, recalculated: exactly what the server will have once it orders the
//! pending patches, so every replica converges.
//!
//! Local edits are found by comparing the document with the last workbook this module saw
//! (`last_seen`), whatever made them. Remote edits are also applied to the undo and redo
//! snapshots, so Undo takes back only the person's own edits and never someone else's.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::Workbook;
use gridcraft_engine::{DocState, Trust};

use crate::patch::{self, DiffCache, WbPatch};
use crate::proto::{BookInfo, LastEdit, Logged, Peer};
use crate::{Error, Result};

pub struct SharedDoc {
    pub book: BookInfo,
    /// [`DocState::uid`] of the open document.
    pub doc_uid: u64,
    confirmed: Arc<Workbook>,
    pub seq: u64,
    pending: VecDeque<(u64, WbPatch)>,
    last_seen: Arc<Workbook>,
    cache: DiffCache,
    next_cseq: u64,
    /// Who last changed each cell's content: (sheet, cell) → (user, time).
    pub last_edits: HashMap<(String, CellRef), (String, i64)>,
    pub peers: Vec<Peer>,
    /// The person may only look (a viewer).
    pub read_only: bool,
    /// Set when this replica can no longer be trusted to match the server; the caller reopens.
    pub needs_reload: Option<String>,
}

/// Something a remote edit did that the front end may want to show.
#[derive(Debug, Default)]
pub struct RemoteEffect {
    pub cells: usize,
    pub user: String,
    /// An edit of the person's that was taken in while applying this one: send it.
    pub outgoing: Option<(u64, WbPatch)>,
}

impl SharedDoc {
    /// A shared workbook as the server sent it: its snapshot (an .xlsx) and the edits since.
    pub fn open(
        book: BookInfo,
        snapshot_seq: u64,
        xlsx: &[u8],
        log: &[Logged],
        last_edits: Vec<LastEdit>,
        peers: Vec<Peer>,
        read_only: bool,
    ) -> Result<(SharedDoc, DocState)> {
        let (mut wb, _warnings) = gridcraft_engine::io::open_bytes("shared.xlsx", xlsx)?;
        let mut seq = snapshot_seq;
        let mut le = HashMap::new();
        for e in last_edits {
            le.insert((e.sheet, CellRef::new(e.r, e.c)), (e.user, e.time));
        }
        for entry in log {
            patch::apply(&mut wb, &entry.patch)?;
            seq = entry.seq;
        }
        let mut doc = DocState::new(wb, None, book.name.clone(), Trust::FromFile);
        // Shared workbooks have no unsaved changes: the server keeps every edit.
        doc.saved = doc.wb.clone();
        let confirmed = doc.wb.clone();
        let shared = SharedDoc {
            doc_uid: doc.uid,
            book,
            last_seen: doc.wb.clone(),
            confirmed,
            seq,
            pending: VecDeque::new(),
            cache: DiffCache::default(),
            next_cseq: 1,
            last_edits: le,
            peers,
            read_only,
            needs_reload: None,
        };
        Ok((shared, doc))
    }

    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Finds the person's edits since the last call. Returns the patch to send (with its client
    /// sequence number), or `None` when nothing shared changed.
    pub fn local_changes(&mut self, d: &mut DocState) -> Result<Option<(u64, WbPatch)>> {
        if Arc::ptr_eq(&d.wb, &self.last_seen) {
            return Ok(None);
        }
        let p = patch::diff(&self.last_seen, &d.wb, &mut self.cache)?;
        if self.read_only && !p.is_empty() {
            // A viewer's edit is taken back at once.
            d.wb = self.last_seen.clone();
            d.calc.rebuild(&d.wb);
            d.revision += 1;
            d.saved = d.wb.clone();
            return Err(Error::Denied("This workbook is view-only for you; ask an administrator for edit access.".into()));
        }
        self.last_seen = d.wb.clone();
        d.saved = d.wb.clone();
        if p.is_empty() {
            return Ok(None);
        }
        let cseq = self.next_cseq;
        self.next_cseq += 1;
        self.pending.push_back((cseq, p.clone()));
        Ok(Some((cseq, p)))
    }

    /// The pending patches, to send again after a reconnect.
    pub fn pending(&self) -> impl Iterator<Item = &(u64, WbPatch)> {
        self.pending.iter()
    }

    /// An edit the server ordered. `cseq` is set when it is this client's own.
    pub fn ordered(&mut self, d: &mut DocState, seq: u64, user: &str, time: i64, cseq: Option<u64>, p: &WbPatch) -> Result<RemoteEffect> {
        if seq <= self.seq {
            return Ok(RemoteEffect::default());
        }
        if seq != self.seq + 1 {
            self.needs_reload = Some(format!("missed edits {}..{}", self.seq + 1, seq - 1));
            return Ok(RemoteEffect::default());
        }
        // The content before this edit, for "last edited by".
        let before = self.confirmed.clone();
        let mut confirmed = (*self.confirmed).clone();
        patch::apply(&mut confirmed, p)?;
        self.confirmed = Arc::new(confirmed);
        self.seq = seq;
        self.book.seq = seq;
        self.note_editors(&before, p, user, time);
        // Our own edit, now ordered: what the person sees already holds it.
        if let Some(c) = cseq
            && let Some(i) = self.pending.iter().position(|(pc, _)| *pc == c)
        {
            self.pending.remove(i);
            return Ok(RemoteEffect::default());
        }
        // Someone else's edit. First take in anything the person did since the last look, so it
        // is pending and survives the rebuild below.
        let outgoing = self.local_changes(d).ok().flatten();
        let mut w = (*self.confirmed).clone();
        let mut applied = patch::Applied::default();
        // What the remote edit wrote, for recalculation.
        let mut probe = (*before).clone();
        let remote = patch::apply(&mut probe, p)?;
        applied.full |= remote.full;
        for (_, pp) in &self.pending {
            let a = patch::apply(&mut w, pp)?;
            applied.full |= a.full;
        }
        let keep_sheet = d.wb.sheet(d.wb.active_sheet()).map(|s| s.name.clone());
        let sheet_index = keep_sheet.and_then(|n| w.sheets.iter().position(|s| s.name == n)).unwrap_or(0);
        w.set_active_sheet(sheet_index.min(w.sheets.len().saturating_sub(1)));
        if applied.full {
            d.calc.rebuild(&w);
            d.calc.recalc_all(&mut w);
        } else {
            // Remote cells by their index in `w` (structure is unchanged, so indices agree).
            d.calc.cells_changed(&mut w, &remote.cells);
        }
        let structure_changed = p.structure.is_some();
        d.wb = Arc::new(w);
        if structure_changed {
            d.sheet_selections.clear();
        }
        // Undo and Redo take back only this person's edits.
        for e in d.undo.iter_mut().chain(d.redo.iter_mut()) {
            let mut hw = (*e.wb).clone();
            if patch::apply(&mut hw, p).is_ok() {
                e.wb = Arc::new(hw);
            }
        }
        // The saved copy is the server's: nothing here is ever "unsaved".
        d.saved = d.wb.clone();
        d.revision += 1;
        self.last_seen = d.wb.clone();
        Ok(RemoteEffect { cells: remote.cells.len(), user: user.to_string(), outgoing })
    }

    /// The server could not order one of our patches: the replica must be reloaded.
    pub fn rejected(&mut self, reason: &str) {
        self.needs_reload = Some(reason.to_string());
    }

    /// After a reconnect: the server's current state replaces `confirmed`, and our edits it never
    /// ordered are applied on top and returned to be sent again. Patches the log shows as
    /// already ordered (`origin`) are dropped.
    pub fn resync(
        &mut self,
        d: &mut DocState,
        snapshot_seq: u64,
        xlsx: &[u8],
        log: &[Logged],
        book: BookInfo,
        applied_cseqs: &[u64],
    ) -> Result<Vec<(u64, WbPatch)>> {
        let (mut wb, _) = gridcraft_engine::io::open_bytes("shared.xlsx", xlsx)?;
        let mut seq = snapshot_seq;
        for entry in log {
            patch::apply(&mut wb, &entry.patch)?;
            seq = entry.seq;
        }
        let _ = self.local_changes(d);
        self.pending.retain(|(c, _)| !applied_cseqs.contains(c));
        self.confirmed = Arc::new(wb.clone());
        for (_, p) in &self.pending {
            patch::apply(&mut wb, p)?;
        }
        let keep_sheet = d.wb.sheet(d.wb.active_sheet()).map(|s| s.name.clone());
        let idx = keep_sheet.and_then(|n| wb.sheets.iter().position(|s| s.name == n)).unwrap_or(0);
        wb.set_active_sheet(idx);
        d.calc.rebuild(&wb);
        d.calc.recalc_all(&mut wb);
        d.wb = Arc::new(wb);
        d.undo.clear();
        d.redo.clear();
        d.sheet_selections.clear();
        d.saved = d.wb.clone();
        d.revision += 1;
        self.last_seen = d.wb.clone();
        self.seq = seq;
        self.book = book;
        self.needs_reload = None;
        Ok(self.pending.iter().cloned().collect())
    }

    fn note_editors(&mut self, before: &Workbook, p: &WbPatch, user: &str, time: i64) {
        for sp in &p.sheets {
            let old_sheet = before.sheets.iter().find(|s| s.name == sp.name);
            for ch in &sp.cells {
                let at = CellRef::new(ch.r, ch.c);
                let old = old_sheet.and_then(|s| s.cells.get(at)).map(|c| (c.formula.as_ref().map(|f| f.text.clone()), c.value.clone(), c.style));
                let edited = match (&ch.cell, &old) {
                    // A formula whose text is unchanged only had its value recalculated.
                    (Some(n), Some((Some(ft), _, _))) => n.f.as_ref().map(|f| &f.text) != Some(ft) || n.f.is_none(),
                    _ => true,
                };
                if edited {
                    self.last_edits.insert((sp.name.clone(), at), (user.to_string(), time));
                }
            }
        }
    }

    pub fn last_edit(&self, sheet: &str, at: CellRef) -> Option<&(String, i64)> {
        self.last_edits.get(&(sheet.to_string(), at))
    }
}
