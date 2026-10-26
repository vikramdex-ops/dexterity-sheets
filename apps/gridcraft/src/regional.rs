//! Regional settings from the operating system, so typed dates and numbers, and the
//! locale-dependent display of number formats (date separator, digit grouping, built-in formats),
//! match Excel on the same computer. Windows: `HKCU\Control Panel\International`. Elsewhere, and
//! when the settings can't be read, GridCraft stays en-US. `DEXSHEETS_LOCALE` (`en-US`, `en-IN`)
//! overrides the system. Saved files never depend on it.

use gridcraft_engine::core::locale::{Locale, set_locale};

/// Sets the process locale. Called once at start-up, before any workbook is read.
pub fn apply() {
    if let Ok(name) = std::env::var("DEXSHEETS_LOCALE")
        && !name.trim().is_empty()
        && !name.trim().eq_ignore_ascii_case("system")
    {
        match Locale::preset(&name) {
            Some(l) => {
                log::info!("regional settings: {} (DEXSHEETS_LOCALE)", l.name);
                set_locale(l);
                return;
            }
            None => log::warn!("DEXSHEETS_LOCALE={name}: unknown locale (en-US, en-IN); using the system settings"),
        }
    }
    if let Some(l) = system() {
        log::info!(
            "regional settings: {} ({:?}, date separator {:?}, decimal {:?}, thousands {:?}, {:?} grouping)",
            l.name,
            l.date_order,
            l.date_sep,
            l.decimal,
            l.thousands,
            l.grouping
        );
        set_locale(l);
    }
}

#[cfg(windows)]
fn system() -> Option<Locale> {
    let key = match windows_registry::CURRENT_USER.open(r"Control Panel\International") {
        Ok(k) => k,
        Err(e) => {
            log::warn!("regional settings unreadable ({e}); using en-US");
            return None;
        }
    };
    Some(Locale::from_windows(|name| key.get_string(name).ok()))
}

#[cfg(not(windows))]
fn system() -> Option<Locale> {
    None
}
