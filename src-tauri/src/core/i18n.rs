//! UI language for messages produced by the backend (errors, notes). Set from settings.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set_language(lang: &str) {
    ENGLISH.store(!lang.eq_ignore_ascii_case("ru"), Ordering::Relaxed);
}

pub fn is_english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// Picks the Russian or English variant of a message.
pub fn tr(ru: &'static str, en: &'static str) -> &'static str {
    if is_english() { en } else { ru }
}

/// `format!` with a Russian and an English template sharing the same arguments.
#[macro_export]
macro_rules! trf {
    ($ru:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if $crate::core::i18n::is_english() { format!($en $(, $arg)*) } else { format!($ru $(, $arg)*) }
    };
}
