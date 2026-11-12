//! Small runtime string table. It deliberately covers the shell, Home and
//! first-run setup first; the remaining pages stay English until their
//! strings are translated, rather than showing machine-translated prose.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language {
    English,
    Malay,
}

impl Language {
    pub fn from_str(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "ms" | "malay" | "bahasa" => Self::Malay,
            _ => Self::English,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Malay => "ms",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Malay => "Bahasa Melayu (beta)",
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(language: Language) {
    CURRENT.store(
        match language {
            Language::English => 0,
            Language::Malay => 1,
        },
        Ordering::Relaxed,
    );
}

pub fn current() -> Language {
    if CURRENT.load(Ordering::Relaxed) == 1 {
        Language::Malay
    } else {
        Language::English
    }
}

/// Pick a string for the active language. Keys are written as the English
/// text so an untranslated call site is obvious in review.
pub fn tr(english: &'static str, malay: &'static str) -> &'static str {
    match current() {
        Language::English => english,
        Language::Malay => malay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_codes_round_trip() {
        assert_eq!(Language::from_str("ms"), Language::Malay);
        assert_eq!(Language::from_str("en"), Language::English);
        assert_eq!(Language::from_str(""), Language::English);
        assert_eq!(Language::Malay.code(), "ms");
    }
}
