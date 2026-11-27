//! Font database setup. Scanning every system font costs ~1 s on Windows, so a
//! curated set is loaded instead and the full scan is only a fallback.

use glyphon::FontSystem;
use glyphon::fontdb::Database;

/// Preferred monospace families, first installed one wins.
#[cfg(windows)]
const MONOSPACE: &[&str] = &["Cascadia Mono", "Consolas", "Courier New"];
#[cfg(target_os = "macos")]
const MONOSPACE: &[&str] = &["SF Mono", "Menlo", "Monaco"];
#[cfg(not(any(windows, target_os = "macos")))]
const MONOSPACE: &[&str] = &["DejaVu Sans Mono", "Noto Sans Mono", "Liberation Mono"];

/// Monospace faces plus the fallbacks cosmic-text asks for (emoji, symbols, CJK, Indic).
#[cfg(windows)]
const CURATED: &[&str] = &[
    "CascadiaMono.ttf",
    "consola.ttf",
    "consolab.ttf",
    "consolai.ttf",
    "consolaz.ttf",
    "segoeui.ttf",
    "seguiemj.ttf",
    "seguisym.ttf",
    "seguihis.ttf",
    "YuGothM.ttc",
    "malgun.ttf",
    "msyh.ttc",
    "Nirmala.ttc",
];

pub(crate) fn font_system(family: Option<&str>) -> FontSystem {
    let mut db = Database::new();
    load_curated(&mut db);
    let missing = match family {
        Some(name) => !has_family(&db, name),
        None => !MONOSPACE.iter().any(|f| has_family(&db, f)),
    };
    if missing {
        db.load_system_fonts();
    }
    if let Some(mono) = MONOSPACE.iter().find(|f| has_family(&db, f)) {
        db.set_monospace_family(*mono);
    }
    log::info!("fonts loaded: {} faces", db.len());
    let locale = sys_locale::get_locale().unwrap_or_else(|| "en-US".to_owned());
    FontSystem::new_with_locale_and_db(locale, db)
}

fn has_family(db: &Database, name: &str) -> bool {
    db.faces().any(|face| {
        face.families
            .iter()
            .any(|(f, _)| f.eq_ignore_ascii_case(name))
    })
}

#[cfg(windows)]
fn load_curated(db: &mut Database) {
    let Some(windir) = std::env::var_os("WINDIR") else {
        return;
    };
    let dir = std::path::Path::new(&windir).join("Fonts");
    for file in CURATED {
        let path = dir.join(file);
        if path.is_file()
            && let Err(e) = db.load_font_file(&path)
        {
            log::warn!("load font {}: {e}", path.display());
        }
    }
}

#[cfg(not(windows))]
fn load_curated(_db: &mut Database) {}
