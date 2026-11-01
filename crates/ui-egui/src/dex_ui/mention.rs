//! @mentions while writing a comment (Dexterity Sheets).
//!
//! Typing `@` in a comment or reply lists the people on the office server; ↑/↓ and Enter or Tab
//! (or a click) put `@Display Name` in the text, Esc closes the list. When the comment reaches
//! the office server, each person mentioned gets a notification that opens the workbook at the
//! comment (`gridcraft_collab::mention`). The list says why nobody would be told: not signed in,
//! or the spreadsheet isn't shared.

use egui::text::{CCursor, CCursorRange};
use egui::{Key, Modifiers, Response, RichText, Ui};
use gridcraft_collab::mention::{self, Person};

use crate::SheetApp;

/// Most people listed at once.
const LIST: usize = 6;
const DIRECTORY: &str = "dex.mention.directory";
const STATE: &str = "dex.mention.state";

/// Who can be mentioned from the active window, refreshed every frame by [`publish`].
#[derive(Clone, Default)]
struct Directory {
    people: Vec<Person>,
    signed_in: bool,
    shared: bool,
}

/// The open suggestion list of one text field.
#[derive(Clone)]
struct Open {
    edit: egui::Id,
    /// Char index of the `@` and of the caret.
    at: usize,
    caret: usize,
    picks: Vec<Person>,
    sel: usize,
    /// Where the list was drawn, so a click on it keeps it open while the field loses focus.
    rect: egui::Rect,
}

/// Esc closed the list for the `@` at this char index in this field.
#[derive(Clone, Copy, PartialEq)]
struct Dismissed(egui::Id, usize);

/// Every frame: the people the comment fields may suggest (everyone on the server but you).
pub fn publish(app: &SheetApp, ctx: &egui::Context) {
    let me = app.collab.me.as_ref().map(|m| m.user.as_str());
    let dir = Directory {
        people: app.collab.users.iter().filter(|u| Some(u.user.as_str()) != me).map(|u| (u.user.clone(), u.display.clone())).collect(),
        signed_in: app.collab.signed_in(),
        shared: crate::collab_ui::active_shared(app).is_some(),
    };
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(DIRECTORY), dir));
}

/// A comment text field with @mention suggestions. `add` draws the field itself; `enabled` is
/// false for notes, which can't mention anyone (as in Excel).
pub fn field(ui: &mut Ui, text: &mut String, enabled: bool, add: impl FnOnce(&mut Ui, &mut String) -> Response) -> Response {
    if !enabled {
        return add(ui, text);
    }
    let ctx = ui.ctx().clone();
    let (sid, did) = (egui::Id::unique(STATE), egui::Id::unique((STATE, "dismissed")));
    let open: Option<Open> = ctx.data(|d| d.get_temp(sid));
    // The list's keys are taken before the field sees them (Enter would end a reply).
    let mut picked: Option<(Open, Person)> = None;
    if let Some(mut o) = open.clone().filter(|o| !o.picks.is_empty() && ctx.memory(|m| m.has_focus(o.edit))) {
        let n = o.picks.len();
        let (mut accept, mut close) = (false, false);
        ui.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                o.sel = (o.sel + 1) % n;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                o.sel = (o.sel + n - 1) % n;
            }
            accept = i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab);
            close = i.consume_key(Modifiers::NONE, Key::Escape);
        });
        if close {
            ctx.data_mut(|d| {
                d.insert_temp(did, Dismissed(o.edit, o.at));
                d.remove::<Open>(sid);
            });
        } else if accept {
            if let Some(p) = o.picks.get(o.sel).cloned() {
                picked = Some((o, p));
            }
        } else {
            ctx.data_mut(|d| d.insert_temp(sid, o));
        }
    }
    if let Some((o, p)) = &picked {
        put(&ctx, text, o, p);
    }
    let mut r = add(ui, text);
    if picked.is_some() {
        r.mark_changed();
    }
    let open: Option<Open> = ctx.data(|d| d.get_temp(sid));
    let over_list = open.as_ref().is_some_and(|o| o.edit == r.id && ctx.pointer_latest_pos().is_some_and(|p| o.rect.contains(p)));
    if !r.has_focus() && !over_list {
        if open.as_ref().is_some_and(|o| o.edit == r.id) {
            ctx.data_mut(|d| d.remove::<Open>(sid));
        }
        return r;
    }
    let caret = egui::text_edit::TextEditState::load(&ctx, r.id).and_then(|s| s.cursor.char_range()).map(|c| usize::from(c.primary.index));
    let typing = caret.and_then(|c| mention::typing(text, c).map(|(at, q)| (at, c, q)));
    let dismissed: Option<Dismissed> = ctx.data(|d| d.get_temp(did));
    let Some((at, caret, query)) = typing.filter(|(at, _, _)| dismissed != Some(Dismissed(r.id, *at))) else {
        ctx.data_mut(|d| d.remove::<Open>(sid));
        return r;
    };
    let dir: Directory = ctx.data(|d| d.get_temp(egui::Id::unique(DIRECTORY))).unwrap_or_default();
    let picks = mention::suggestions(&query, &dir.people, LIST);
    // Nobody matches: the `@` was something else (an address, "@ 5 m"), so say nothing.
    if picks.is_empty() && (dir.signed_in || !query.is_empty()) {
        ctx.data_mut(|d| d.remove::<Open>(sid));
        return r;
    }
    let sel = open.as_ref().filter(|o| o.edit == r.id && o.at == at).map_or(0, |o| o.sel.min(picks.len().saturating_sub(1)));
    let mut clicked: Option<Person> = None;
    let area = egui::Area::new(egui::Id::unique((STATE, r.id)))
        .order(egui::Order::Foreground)
        .fixed_pos(r.rect.left_bottom() + egui::vec2(0.0, 2.0))
        .show(&ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(r.rect.width().clamp(200.0, 320.0));
                if !dir.signed_in {
                    ui.label(RichText::new("Sign in to the office server (File › Shared) to mention people.").weak());
                    return;
                }
                for (i, p) in picks.iter().enumerate() {
                    let line = if p.1.eq_ignore_ascii_case(&p.0) { p.1.clone() } else { format!("{}  ·  {}", p.1, p.0) };
                    if ui.selectable_label(i == sel, line).clicked() {
                        clicked = Some(p.clone());
                    }
                }
                if !dir.shared {
                    ui.separator();
                    ui.label(RichText::new("This spreadsheet isn't shared, so the people you mention won't be told. Share it first.").small().weak());
                }
            });
        });
    let o = Open { edit: r.id, at, caret, picks, sel, rect: area.response.rect };
    if let Some(p) = clicked {
        put(&ctx, text, &o, &p);
        r.mark_changed();
        r.request_focus();
        ctx.data_mut(|d| d.remove::<Open>(sid));
    } else {
        ctx.data_mut(|d| d.insert_temp(sid, o));
    }
    r
}

/// Replaces the `@query` with the person's name and puts the caret after it.
fn put(ctx: &egui::Context, text: &mut String, o: &Open, p: &Person) {
    let (next, end) = mention::insert(text, o.at, o.caret, &p.1);
    *text = next;
    let mut st = egui::text_edit::TextEditState::load(ctx, o.edit).unwrap_or_default();
    st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(end))));
    st.store(ctx, o.edit);
}
