//! @mentions in comments (Dexterity Sheets).
//!
//! A comment or reply that names a colleague as `@Display Name` (or `@username`) tells that
//! person: the server finds mentions new in an edit ([`new_mentions`]) and sends each person a
//! notification that opens the workbook at the comment. The app suggests people while `@` is
//! typed ([`typing`], [`suggestions`]). Mentions are plain text, so they survive saving as .xlsx
//! and read the same in Excel.

use std::collections::HashMap;

use gridcraft_engine::model::{Comment, Workbook};

use crate::patch::{Slot, WbPatch};

/// A person who can be mentioned: (user name, display name).
pub type Person = (String, String);

/// Longest run after `@` the app looks at while suggesting (display names have spaces).
const QUERY_MAX: usize = 40;
/// Longest part of a comment quoted in a notification.
const QUOTE_MAX: usize = 140;

/// The people mentioned in `text`, in order, once each. At each `@` (at the start or after a
/// space or punctuation, so `name@example.com` is not one) the longest display name or user
/// name that follows, ignoring case and ending at a word boundary, is the person.
pub fn mentioned(text: &str, people: &[Person]) -> Vec<String> {
    let lower = text.to_lowercase();
    // Lowercasing can change byte lengths for a few scripts; positions are only trusted when it doesn't.
    let hay = if lower.len() == text.len() { lower } else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    for (i, _) in hay.match_indices('@') {
        if hay[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.') {
            continue;
        }
        let rest = &hay[i + 1..];
        // (length, is a display name, user): the longest name wins; on a tie a display name beats
        // someone else's user name, since that is what the app inserts.
        let mut best: Option<(usize, bool, &str)> = None;
        for (user, display) in people {
            for (is_display, name) in [(true, display.as_str()), (false, user.as_str())] {
                let n = name.trim().to_lowercase();
                if n.is_empty() || !rest.starts_with(&n) {
                    continue;
                }
                if rest[n.len()..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    continue;
                }
                if best.is_none_or(|(l, d, _)| (n.len(), is_display) > (l, d)) {
                    best = Some((n.len(), is_display, user.as_str()));
                }
            }
        }
        if let Some((_, _, u)) = best
            && !out.iter().any(|x| x == u)
        {
            out.push(u.to_string());
        }
    }
    out
}

/// A comment or reply with people mentioned in it for the first time.
#[derive(Clone, Debug, PartialEq)]
pub struct Mention {
    pub sheet: String,
    pub cell: String,
    /// The comment or reply, shortened for a notification.
    pub quote: String,
    /// User names, in the order they appear.
    pub users: Vec<String>,
}

/// The mentions `patch` adds, comparing the workbook before and after it. Only comments and
/// replies that are new or edited count, and only people not already mentioned in that text
/// before: inserting rows (comments move), renaming a sheet or editing a comment never repeats
/// a notification. Sheets the edit creates (a copied sheet brings its comments along) are left
/// out for the same reason.
pub fn new_mentions(before: &Workbook, after: &Workbook, patch: &WbPatch, people: &[Person]) -> Vec<Mention> {
    let mut out = Vec::new();
    for sp in &patch.sheets {
        if !sp.fields.contains_key("comments") && !sp.keyed.contains_key("comments") {
            continue;
        }
        let was_named = match &patch.structure {
            Some(st) => match st.slots.iter().find(|s| matches!(s, Slot::Existing { name, .. } | Slot::New { name } if *name == sp.name)) {
                Some(Slot::Existing { from, .. }) => from.clone(),
                Some(Slot::New { .. }) => continue,
                None => sp.name.clone(),
            },
            None => sp.name.clone(),
        };
        let (Some(old), Some(new)) = (before.sheets.iter().find(|s| s.name == was_named), after.sheets.iter().find(|s| s.name == sp.name)) else {
            continue;
        };
        // Every text the sheet's comments held before, so a comment that only moved is not new.
        let mut had: HashMap<&str, usize> = HashMap::new();
        for c in old.comments.values() {
            for t in entries(c) {
                *had.entry(t).or_default() += 1;
            }
        }
        for (at, c) in &new.comments {
            let prior = old.comments.get(at);
            for (k, t) in entries(c).enumerate() {
                if let Some(n) = had.get_mut(t).filter(|n| **n > 0) {
                    *n -= 1;
                    continue;
                }
                let already = prior.and_then(|p| entries(p).nth(k)).map(|p| mentioned(p, people)).unwrap_or_default();
                let users: Vec<String> = mentioned(t, people).into_iter().filter(|u| !already.contains(u)).collect();
                if !users.is_empty() {
                    out.push(Mention { sheet: sp.name.clone(), cell: at.a1(), quote: quote(t), users });
                }
            }
        }
    }
    out
}

/// A comment's texts: the comment itself, then each reply.
fn entries(c: &Comment) -> impl Iterator<Item = &str> {
    std::iter::once(c.text.as_str()).chain(c.replies.iter().map(|(_, t)| t.as_str()))
}

fn quote(t: &str) -> String {
    let one_line = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= QUOTE_MAX {
        one_line
    } else {
        format!("{}…", one_line.chars().take(QUOTE_MAX - 1).collect::<String>().trim_end())
    }
}

/// The `@name` being typed when the caret is at `caret` (a char index): the char index of the
/// `@` and what follows it up to the caret. `None` when the caret is not in one.
pub fn typing(text: &str, caret: usize) -> Option<(usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let caret = caret.min(chars.len());
    let lo = caret.saturating_sub(QUERY_MAX + 1);
    let at = (lo..caret).rev().find(|&i| chars.get(i) == Some(&'@'))?;
    if at > 0 && chars.get(at - 1).is_some_and(|c| c.is_alphanumeric() || *c == '_' || *c == '.') {
        return None;
    }
    let query: String = chars.get(at + 1..caret)?.iter().collect();
    if query.contains('\n') || query.starts_with(' ') || query.contains("  ") {
        return None;
    }
    Some((at, query))
}

/// People whose display name (any word of it) or user name starts with `query`, ignoring case,
/// best first: a display name that starts with it, then a later word, then the user name.
/// An empty query lists everyone, by display name.
pub fn suggestions(query: &str, people: &[Person], limit: usize) -> Vec<Person> {
    let q = query.trim().to_lowercase();
    let mut ranked: Vec<(u8, &Person)> = people
        .iter()
        .filter_map(|p| {
            let d = p.1.to_lowercase();
            let rank = if d.starts_with(&q) {
                0
            } else if d.split_whitespace().skip(1).any(|w| w.starts_with(&q)) || (q.contains(' ') && d.contains(&q)) {
                1
            } else if p.0.to_lowercase().starts_with(&q) {
                2
            } else {
                return None;
            };
            Some((rank, p))
        })
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.1.to_lowercase().cmp(&b.1.1.to_lowercase())));
    ranked.into_iter().take(limit).map(|(_, p)| p.clone()).collect()
}

/// `text` with the `@query` that starts at char `at` and ends at char `caret` replaced by
/// `@Display Name ` (the space ends the mention), and the char index after it.
pub fn insert(text: &str, at: usize, caret: usize, display: &str) -> (String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let (at, caret) = (at.min(chars.len()), caret.min(chars.len()).max(at.min(chars.len())));
    let mut out: String = chars[..at].iter().collect();
    out.push('@');
    out.push_str(display.trim());
    let next = chars.get(caret);
    if next.is_none_or(|c| !c.is_whitespace()) {
        out.push(' ');
    }
    let end = out.chars().count() + usize::from(next.is_some_and(|c| c.is_whitespace()));
    out.extend(chars[caret..].iter());
    (out, end)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gridcraft_engine::core::CellRef;

    fn people() -> Vec<Person> {
        vec![
            ("anita".into(), "Anita Rao".into()),
            ("anu".into(), "Anita".into()),
            ("rahul".into(), "Rahul".into()),
            ("vikram".into(), "Vikram".into()),
        ]
    }

    #[test]
    fn finds_people_by_display_or_user_name_longest_first() {
        let p = people();
        assert_eq!(mentioned("@Anita Rao please check the length", &p), vec!["anita"]);
        assert_eq!(mentioned("@anita can you look", &p), vec!["anu"], "\"Anita\" is a display name: the longest name at that @ wins");
        assert_eq!(mentioned("cc @rahul, @VIKRAM and @rahul again", &p), vec!["rahul", "vikram"]);
        assert_eq!(mentioned("thanks @Rahul.", &p), vec!["rahul"]);
        assert!(mentioned("mail rahul@example.com", &p).is_empty(), "an e-mail address is not a mention");
        assert!(mentioned("@Rahulx and @nobody", &p).is_empty(), "a longer word is someone else");
        assert!(mentioned("no mention here", &p).is_empty());
    }

    fn book(comments: &[(&str, &str, &[&str])]) -> Workbook {
        let mut wb = Workbook::new();
        if let Some(s) = wb.sheets.first_mut() {
            std::sync::Arc::make_mut(s).comments = comments
                .iter()
                .map(|(at, text, replies)| {
                    (
                        CellRef::parse(at).expect("cell"),
                        Comment {
                            author: "Vikram".into(),
                            text: (*text).into(),
                            replies: replies.iter().map(|r| ("Anita Rao".to_string(), (*r).to_string())).collect(),
                            threaded: true,
                            resolved: false,
                            visible: false,
                            dates: vec![],
                            note_box: None,
                        },
                    )
                })
                .collect();
        }
        wb
    }

    fn diff(a: &Workbook, b: &Workbook) -> Vec<Mention> {
        let patch = crate::patch::diff(&std::sync::Arc::new(a.clone()), &std::sync::Arc::new(b.clone()), &mut Default::default()).expect("diff");
        new_mentions(a, b, &patch, &people())
    }

    #[test]
    fn only_new_or_edited_text_mentions_anyone() {
        let empty = book(&[]);
        let one = book(&[("B4", "@Rahul please check the length", &[])]);
        let m = diff(&empty, &one);
        assert_eq!(m.len(), 1);
        assert_eq!(
            (m[0].cell.as_str(), m[0].users.clone(), m[0].quote.as_str()),
            ("B4", vec!["rahul".to_string()], "@Rahul please check the length")
        );
        // A reply mentioning someone else tells only them.
        let replied = book(&[("B4", "@Rahul please check the length", &["done, @Vikram"])]);
        assert_eq!(diff(&one, &replied).iter().flat_map(|m| m.users.clone()).collect::<Vec<_>>(), vec!["vikram"]);
        // Editing the comment keeps Rahul (already told) and adds Vikram.
        let edited = book(&[("B4", "@Rahul and @Vikram please check the length", &[])]);
        assert_eq!(diff(&one, &edited).iter().flat_map(|m| m.users.clone()).collect::<Vec<_>>(), vec!["vikram"]);
        // Rows inserted above: the comment moves to B5 and nobody is told again.
        let moved = book(&[("B5", "@Rahul please check the length", &[])]);
        assert!(diff(&one, &moved).is_empty());
        // Resolving or deleting tells nobody.
        assert!(diff(&one, &empty).is_empty());
    }

    #[test]
    fn suggests_while_typing_and_inserts_the_name() {
        assert_eq!(typing("please @Ani", 11), Some((7, "Ani".into())));
        assert_eq!(typing("@", 1), Some((0, String::new())));
        assert_eq!(typing("a@b", 3), None, "inside a word");
        assert_eq!(typing("@Anita Rao done", 15), Some((0, "Anita Rao done".into())), "the list decides: nobody matches this");
        assert_eq!(typing("@x\ny", 4), None);
        assert_eq!(typing("no at", 5), None);
        let p = people();
        let names = |q: &str| suggestions(q, &p, 6).into_iter().map(|x| x.1).collect::<Vec<_>>();
        assert_eq!(names("ani"), vec!["Anita", "Anita Rao"]);
        assert_eq!(names("rao"), vec!["Anita Rao"]);
        assert_eq!(names("vik"), vec!["Vikram"]);
        assert_eq!(names("anita r"), vec!["Anita Rao"]);
        assert_eq!(names("").len(), 4);
        assert!(names("anita rao done").is_empty());
        assert_eq!(insert("please @Ani", 7, 11, "Anita Rao"), ("please @Anita Rao ".to_string(), 18));
        assert_eq!(insert("@Ra check", 0, 3, "Rahul"), ("@Rahul check".to_string(), 7));
    }
}
