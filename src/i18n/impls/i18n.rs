//! Tiny runtime internationalisation for the headless build.
//!
//! All user-facing strings in the CLI/MCP paths are built in Rust, so the
//! selected language is a single process-wide flag. The desktop build also fed
//! this into Slint's bundled `.po` translations; that mechanism went away with
//! the GUI. Select a language with `MEATSHELL_LANG=en` (default is Chinese).

use std::sync::atomic::{AtomicU8, Ordering};

const ZH: u8 = 0;
const EN: u8 = 1;

static LANG: AtomicU8 = AtomicU8::new(ZH);

/// Apply a language code (`"zh"` or `"en"`). Anything else keeps the default.
pub fn set_language(code: &str) {
    let en = code.eq_ignore_ascii_case("en");
    LANG.store(if en { EN } else { ZH }, Ordering::Relaxed);
}

/// Current language code, for reporting or persisting.
pub fn current_code() -> &'static str {
    if is_en() {
        "en"
    } else {
        "zh"
    }
}

pub fn is_en() -> bool {
    LANG.load(Ordering::Relaxed) == EN
}

/// Pick the variant for the current language: `zh` is Chinese, `en` is English.
pub fn t(zh: &'static str, en: &'static str) -> &'static str {
    if is_en() {
        en
    } else {
        zh
    }
}
