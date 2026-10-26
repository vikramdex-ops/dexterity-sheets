//! Workbook patches: what one edit changed, as absolute assignments that every replica applies
//! in the server's order.
//!
//! A patch is computed by comparing two workbooks (`diff`), so it captures every kind of change
//! (commands, undo, paste, the control channel) without knowing which command made it. Every
//! entry is "this key now holds this content", never "add one to this", so applying the same
//! patches in the same order gives the same workbook on every machine, and re-applying a
//! client's own unacknowledged patches on top of a remote one keeps the client's edits.
//!
//! Granularity: one cell; one top-level field of a sheet or the workbook (an object-valued field
//! such as column widths is split per key); the sheet list. Cell styles travel as the style
//! itself rather than its id, because style ids are indices into each replica's own table.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use gridcraft_engine::core::{CellRef, RangeRef, Value};
use gridcraft_engine::model::{Cell, CellStore, Formula, Sheet, Style, StyleId, Workbook};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

use crate::{Error, Result};

/// Sheet fields that belong to the person looking at the sheet, not to the shared document.
const SHEET_LOCAL: &[&str] = &["cells", "name", "view_active", "view_top_left", "zoom", "view"];
/// Workbook fields that are not compared as JSON: sheets and styles have their own handling, the
/// active sheet is each person's own, and the macro blob never changes in this app.
const WORKBOOK_LOCAL: &[&str] = &["sheets", "styles", "active_sheet", "vba_project"];

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WbPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structure: Option<Structure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sheets: Vec<SheetPatch>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub fields: Map<String, Json>,
}

/// The sheet list after the edit. `base` is the list the edit started from, so a sheet another
/// person added meanwhile (not in `base`) is kept rather than deleted.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Structure {
    pub base: Vec<String>,
    pub slots: Vec<Slot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "lowercase")]
pub enum Slot {
    /// A sheet that existed before (by its name then), now called `name`.
    Existing { from: String, name: String },
    /// A sheet the edit created; its content follows in a [`SheetPatch`].
    New { name: String },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SheetPatch {
    /// The sheet's name after the edit (and after `structure`).
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<CellChange>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub fields: Map<String, Json>,
    /// Object-valued fields changed key by key: `None` removes the key.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keyed: BTreeMap<String, BTreeMap<String, Option<Json>>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellChange {
    pub r: u32,
    pub c: u32,
    /// `None` = the cell was cleared entirely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<CellData>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellData {
    #[serde(default, skip_serializing_if = "Value::is_empty")]
    pub v: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub f: Option<FormulaData>,
    /// `None` = the default style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s: Option<Style>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormulaData {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub array: Option<RangeRef>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dynamic: bool,
}

impl WbPatch {
    pub fn is_empty(&self) -> bool {
        self.structure.is_none() && self.fields.is_empty() && self.sheets.iter().all(SheetPatch::is_empty)
    }
    /// Number of cells the patch writes.
    pub fn cell_count(&self) -> usize {
        self.sheets.iter().map(|s| s.cells.len()).sum()
    }
}

impl SheetPatch {
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty() && self.fields.is_empty() && self.keyed.is_empty()
    }
}

impl CellData {
    fn from_cell(cell: &Cell, wb: &Workbook) -> CellData {
        CellData {
            v: cell.value.clone(),
            f: cell.formula.as_ref().map(|f| FormulaData { text: f.text.clone(), array: f.array, dynamic: f.dynamic }),
            s: (cell.style != StyleId::DEFAULT).then(|| wb.styles.get(cell.style).clone()),
        }
    }
    /// What the person typed, for the audit log: the formula with `=`, or the value.
    pub fn input(&self) -> String {
        match &self.f {
            Some(f) => format!("={}", f.text),
            None => value_text(&self.v),
        }
    }
    /// Whether this is a person's edit rather than a recalculated result: a constant or a
    /// formula (a formula cell whose text and style are unchanged only had its value recomputed).
    pub fn same_content(&self, other: &CellData) -> bool {
        match (&self.f, &other.f) {
            (Some(a), Some(b)) => a == b && self.s == other.s,
            (None, None) => self == other,
            _ => false,
        }
    }
}

pub fn value_text(v: &Value) -> String {
    match v {
        Value::Empty => String::new(),
        Value::Text(t) => t.to_string(),
        other => serde_json::to_value(other).ok().and_then(|j| j.get("v").map(|x| x.to_string().trim_matches('"').to_string())).unwrap_or_default(),
    }
}

/// The JSON of a sheet without its cells and without the viewer's own state.
pub fn sheet_fields(sheet: &Sheet) -> Result<Map<String, Json>> {
    let mut bare = sheet.clone();
    bare.cells = CellStore::new();
    match serde_json::to_value(&bare)? {
        Json::Object(mut m) => {
            for k in SHEET_LOCAL {
                m.remove(*k);
            }
            Ok(m)
        }
        _ => Err(Error::Protocol("sheet did not serialize as an object".into())),
    }
}

/// The JSON of a workbook without its sheets, styles and the viewer's own state.
pub fn workbook_fields(wb: &Workbook) -> Result<Map<String, Json>> {
    let mut bare = wb.clone();
    bare.sheets.clear();
    bare.styles = Default::default();
    bare.vba_project = None;
    match serde_json::to_value(&bare)? {
        Json::Object(mut m) => {
            for k in WORKBOOK_LOCAL {
                m.remove(*k);
            }
            Ok(m)
        }
        _ => Err(Error::Protocol("workbook did not serialize as an object".into())),
    }
}

/// Remembers the field JSON of sheets already compared, so a diff serializes only the sheets an
/// edit replaced. Holds the sheets themselves, so a pointer is never reused while cached.
#[derive(Default)]
pub struct DiffCache {
    sheets: Vec<(Arc<Sheet>, Arc<Map<String, Json>>)>,
    workbook: Option<(Arc<Workbook>, Arc<Map<String, Json>>)>,
}

impl DiffCache {
    fn sheet(&self, s: &Arc<Sheet>) -> Option<Arc<Map<String, Json>>> {
        self.sheets.iter().find(|(k, _)| Arc::ptr_eq(k, s)).map(|(_, m)| m.clone())
    }
    fn sheet_fields(&mut self, s: &Arc<Sheet>, fresh: &mut Vec<(Arc<Sheet>, Arc<Map<String, Json>>)>) -> Result<Arc<Map<String, Json>>> {
        let m = match self.sheet(s) {
            Some(m) => m,
            None => Arc::new(sheet_fields(s)?),
        };
        fresh.push((s.clone(), m.clone()));
        Ok(m)
    }
    fn workbook_fields(&self, wb: &Arc<Workbook>) -> Result<Arc<Map<String, Json>>> {
        match &self.workbook {
            Some((k, m)) if Arc::ptr_eq(k, wb) => Ok(m.clone()),
            _ => Ok(Arc::new(workbook_fields(wb)?)),
        }
    }
}

/// What changed from `old` to `new`.
pub fn diff(old: &Arc<Workbook>, new: &Arc<Workbook>, cache: &mut DiffCache) -> Result<WbPatch> {
    let mut patch = WbPatch::default();
    if Arc::ptr_eq(old, new) {
        return Ok(patch);
    }
    // Workbook-level fields.
    let (of, nf) = (cache.workbook_fields(old)?, cache.workbook_fields(new)?);
    for (k, v) in nf.iter() {
        if of.get(k) != Some(v) {
            patch.fields.insert(k.clone(), v.clone());
        }
    }
    for k in of.keys() {
        if !nf.contains_key(k) {
            patch.fields.insert(k.clone(), Json::Null);
        }
    }
    cache.workbook = Some((new.clone(), nf));

    // Pair each new sheet with the old sheet it came from.
    let mut used = vec![false; old.sheets.len()];
    let mut origin: Vec<Option<usize>> = vec![None; new.sheets.len()];
    for (j, s) in new.sheets.iter().enumerate() {
        if let Some(i) = old.sheets.iter().enumerate().position(|(i, o)| !used.get(i).copied().unwrap_or(true) && Arc::ptr_eq(o, s)) {
            used[i] = true;
            origin[j] = Some(i);
        }
    }
    for (j, s) in new.sheets.iter().enumerate() {
        if origin[j].is_some() {
            continue;
        }
        if let Some(i) = old.sheets.iter().enumerate().position(|(i, o)| !used[i] && o.name == s.name) {
            used[i] = true;
            origin[j] = Some(i);
        }
    }
    // One sheet left on each side: a rename (with or without other edits).
    let free_new: Vec<usize> = (0..new.sheets.len()).filter(|j| origin[*j].is_none()).collect();
    let free_old: Vec<usize> = (0..old.sheets.len()).filter(|i| !used[*i]).collect();
    if let ([j], [i]) = (free_new.as_slice(), free_old.as_slice()) {
        used[*i] = true;
        origin[*j] = Some(*i);
    }
    let same_list = old.sheets.len() == new.sheets.len()
        && origin.iter().enumerate().all(|(j, o)| *o == Some(j))
        && old.sheets.iter().zip(&new.sheets).all(|(a, b)| a.name == b.name);
    if !same_list {
        patch.structure = Some(Structure {
            base: old.sheets.iter().map(|s| s.name.clone()).collect(),
            slots: new
                .sheets
                .iter()
                .zip(&origin)
                .map(|(s, o)| match o.and_then(|i| old.sheets.get(i)) {
                    Some(from) => Slot::Existing { from: from.name.clone(), name: s.name.clone() },
                    None => Slot::New { name: s.name.clone() },
                })
                .collect(),
        });
    }

    let mut fresh = Vec::new();
    let empty = Arc::new(Sheet::new(String::new()));
    for (j, s) in new.sheets.iter().enumerate() {
        let before = origin[j].and_then(|i| old.sheets.get(i)).unwrap_or(&empty);
        if Arc::ptr_eq(before, s) {
            if let Some(m) = cache.sheet(s) {
                fresh.push((s.clone(), m));
            }
            continue;
        }
        let mut sp = SheetPatch { name: s.name.clone(), ..Default::default() };
        for at in before.cells.diff(&s.cells) {
            let cell = s.cells.get(at).map(|c| CellData::from_cell(c, new));
            // A style-id change that resolves to the same style is not a change.
            if let (Some(a), Some(b)) = (&cell, before.cells.get(at).map(|c| CellData::from_cell(c, old)))
                && *a == b
            {
                continue;
            }
            sp.cells.push(CellChange { r: at.row, c: at.col, cell });
        }
        let ofields = cache.sheet_fields(before, &mut Vec::new())?;
        let nfields = cache.sheet_fields(s, &mut fresh)?;
        for (k, v) in nfields.iter() {
            let was = ofields.get(k);
            if was == Some(v) {
                continue;
            }
            match (was, v) {
                (Some(Json::Object(a)), Json::Object(b)) => {
                    let mut keys = BTreeMap::new();
                    for (kk, vv) in b {
                        if a.get(kk) != Some(vv) {
                            keys.insert(kk.clone(), Some(vv.clone()));
                        }
                    }
                    for kk in a.keys() {
                        if !b.contains_key(kk) {
                            keys.insert(kk.clone(), None);
                        }
                    }
                    sp.keyed.insert(k.clone(), keys);
                }
                _ => {
                    sp.fields.insert(k.clone(), v.clone());
                }
            }
        }
        if !sp.is_empty() || origin[j].is_none() {
            patch.sheets.push(sp);
        }
    }
    cache.sheets = fresh;
    Ok(patch)
}

/// What applying a patch touched, for recalculation.
#[derive(Debug, Default)]
pub struct Applied {
    /// Cells written, as (sheet index after the patch, cell).
    pub cells: Vec<(usize, CellRef)>,
    /// The sheet list or a field formulas depend on changed: rebuild and recalculate everything.
    pub full: bool,
}

/// Applies `patch` to `wb`. Keys that no longer exist (a sheet someone else deleted meanwhile)
/// are skipped: the server's order already decided that the deletion wins.
pub fn apply(wb: &mut Workbook, patch: &WbPatch) -> Result<Applied> {
    let mut out = Applied::default();
    if let Some(st) = &patch.structure {
        apply_structure(wb, st);
        out.full = true;
    }
    if !patch.fields.is_empty() {
        let mut obj = workbook_fields(wb)?;
        for (k, v) in &patch.fields {
            if WORKBOOK_LOCAL.contains(&k.as_str()) {
                continue;
            }
            if v.is_null() {
                obj.remove(k);
            } else {
                obj.insert(k.clone(), v.clone());
            }
        }
        obj.insert("sheets".into(), Json::Array(Vec::new()));
        obj.insert("styles".into(), serde_json::to_value(gridcraft_engine::model::StyleTable::default())?);
        obj.insert("active_sheet".into(), serde_json::to_value(wb.active_sheet())?);
        let mut nw: Workbook = serde_json::from_value(Json::Object(obj))?;
        nw.sheets = std::mem::take(&mut wb.sheets);
        nw.styles = std::mem::take(&mut wb.styles);
        nw.vba_project = wb.vba_project.take();
        nw.preserved = wb.preserved.take();
        nw.source_dir = wb.source_dir.take();
        nw.source_path = wb.source_path.take();
        nw.set_active_sheet(wb.active_sheet());
        hydrate_names(&mut nw);
        *wb = nw;
        out.full = true;
    }
    for sp in &patch.sheets {
        let Some(idx) = wb.sheets.iter().position(|s| s.name == sp.name) else { continue };
        // Styles first: interning needs the workbook, the cells need the sheet.
        let styles: Vec<StyleId> = sp
            .cells
            .iter()
            .map(|c| match c.cell.as_ref().and_then(|d| d.s.clone()) {
                Some(s) => wb.styles.intern(s),
                None => StyleId::DEFAULT,
            })
            .collect();
        if !sp.fields.is_empty() || !sp.keyed.is_empty() {
            apply_sheet_fields(wb, idx, sp)?;
            out.full = true;
        }
        let Some(sheet) = wb.sheet_mut(idx) else { continue };
        for (ch, style) in sp.cells.iter().zip(styles) {
            let at = CellRef::new(ch.r, ch.c);
            match &ch.cell {
                None => {
                    sheet.cells.remove(at);
                }
                Some(d) => {
                    let formula = d.f.as_ref().map(|f| {
                        let mut fm = Formula::new(&f.text);
                        fm.array = f.array;
                        fm.dynamic = f.dynamic;
                        Arc::new(fm)
                    });
                    sheet.cells.set(at, Cell { value: d.v.clone(), formula, style });
                }
            }
            out.cells.push((idx, at));
        }
    }
    Ok(out)
}

fn apply_structure(wb: &mut Workbook, st: &Structure) {
    let active_name = wb.sheet(wb.active_sheet()).map(|s| s.name.clone());
    let mut pool: Vec<Option<Arc<Sheet>>> = wb.sheets.drain(..).map(Some).collect();
    let take = |pool: &mut Vec<Option<Arc<Sheet>>>, name: &str| -> Option<Arc<Sheet>> {
        let i = pool.iter().position(|s| s.as_ref().is_some_and(|s| s.name == name))?;
        pool.get_mut(i).and_then(Option::take)
    };
    let mut sheets: Vec<Arc<Sheet>> = Vec::with_capacity(st.slots.len());
    for slot in &st.slots {
        match slot {
            Slot::Existing { from, name } => {
                // A sheet deleted by someone else before this edit reached the server stays deleted.
                let Some(mut s) = take(&mut pool, from) else { continue };
                if s.name != *name {
                    Arc::make_mut(&mut s).name = name.clone();
                }
                sheets.push(s);
            }
            Slot::New { name } => sheets.push(Arc::new(Sheet::new(name.clone()))),
        }
    }
    // Sheets this edit never saw (added by someone else meanwhile) stay, after the others.
    for s in pool.into_iter().flatten() {
        if !st.base.contains(&s.name) {
            let mut s = s;
            if sheets.iter().any(|t| t.name == s.name) {
                let base = s.name.clone();
                let name = (2..).map(|n| format!("{base} ({n})")).find(|n| !sheets.iter().any(|t| &t.name == n)).unwrap_or(base);
                Arc::make_mut(&mut s).name = name;
            }
            sheets.push(s);
        }
    }
    if sheets.is_empty() {
        sheets.push(Arc::new(Sheet::new("Sheet1")));
    }
    wb.sheets = sheets;
    let active = active_name.and_then(|n| wb.sheets.iter().position(|s| s.name == n)).unwrap_or(0);
    wb.set_active_sheet(active.min(wb.sheets.len().saturating_sub(1)));
}

fn apply_sheet_fields(wb: &mut Workbook, idx: usize, sp: &SheetPatch) -> Result<()> {
    let Some(sheet) = wb.sheets.get(idx) else { return Ok(()) };
    let mut obj = serde_json::to_value({
        let mut bare = (**sheet).clone();
        bare.cells = CellStore::new();
        bare
    })?;
    let Json::Object(m) = &mut obj else { return Err(Error::Protocol("sheet did not serialize as an object".into())) };
    for (k, v) in &sp.fields {
        if SHEET_LOCAL.contains(&k.as_str()) {
            continue;
        }
        if v.is_null() {
            m.remove(k);
        } else {
            m.insert(k.clone(), v.clone());
        }
    }
    for (k, keys) in &sp.keyed {
        if SHEET_LOCAL.contains(&k.as_str()) {
            continue;
        }
        let entry = m.entry(k.clone()).or_insert_with(|| Json::Object(Map::new()));
        if !entry.is_object() {
            *entry = Json::Object(Map::new());
        }
        if let Json::Object(target) = entry {
            for (kk, vv) in keys {
                match vv {
                    Some(v) => {
                        target.insert(kk.clone(), v.clone());
                    }
                    None => {
                        target.remove(kk);
                    }
                }
            }
        }
    }
    let mut fresh: Sheet = serde_json::from_value(obj)?;
    let Some(old) = wb.sheet_mut(idx) else { return Ok(()) };
    fresh.cells = std::mem::take(&mut old.cells);
    fresh.spill = std::mem::take(&mut old.spill);
    fresh.spill_ranges = std::mem::take(&mut old.spill_ranges);
    fresh.preserved = old.preserved.take();
    fresh.file_id = old.file_id;
    fresh.name = old.name.clone();
    fresh.view_active = old.view_active;
    fresh.view_top_left = old.view_top_left;
    fresh.zoom = old.zoom;
    fresh.view = old.view;
    *old = fresh;
    Ok(())
}

/// Defined names deserialized from JSON carry their formula text only; nothing to do unless
/// the model keeps a parsed form. Kept as one place to add it if that changes.
fn hydrate_names(_wb: &mut Workbook) {}

/// The sheets a patch touches by name, for logging.
pub fn touched_sheets(p: &WbPatch) -> BTreeSet<String> {
    p.sheets.iter().map(|s| s.name.clone()).collect()
}
