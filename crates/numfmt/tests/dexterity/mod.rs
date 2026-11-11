//! Dexterity Sheets: the rows of official GridCraft's golden files that are different here on
//! purpose. Upstream's files (`tests/golden/*.txt`) stay exactly as upstream records them, so a
//! sync never has to merge them; `codes.txt` in this folder holds only our rows, and
//! `render_equality.rs` compares against upstream's golden with those rows replaced (one call,
//! placed by packaging/dexterity/reapply_overlay.py).
//!
//! What differs: the section a conditional number format picks and when it shows the minus sign,
//! which follow what Excel shows (crates/numfmt/src/tests.rs, `conditional_sections_match_excel`).
//! Every other row must match upstream's digest; a row listed here that upstream no longer has,
//! or that now matches upstream anyway, fails the test so the list never goes stale.

/// Upstream's golden text for `name` with Dexterity's rows in place of upstream's.
pub fn golden(name: &str, upstream: &str) -> String {
    let ours: Vec<&str> = if name == "codes" {
        include_str!("codes.txt").lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).collect()
    } else {
        Vec::new()
    };
    let key = |line: &str| line.splitn(3, ' ').nth(2).unwrap_or_default().to_string();
    let mut used = vec![false; ours.len()];
    let mut out = String::new();
    for line in upstream.lines() {
        match ours.iter().position(|o| key(o) == key(line)) {
            Some(i) => {
                assert_ne!(ours[i], line, "tests/dexterity/codes.txt lists a row that matches official GridCraft again: remove it");
                used[i] = true;
                out.push_str(ours[i]);
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    for (row, used) in ours.iter().zip(used) {
        assert!(used, "tests/dexterity/codes.txt lists a row official GridCraft's golden no longer has: {row}");
    }
    out
}
