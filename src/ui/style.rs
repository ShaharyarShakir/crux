//! ANSI styling that collapses to plain text when color is disabled.
//!
//! Semantic colors (error, warning, note, info, success) use the 16 standard
//! ANSI colors so they follow the user's terminal theme. When the terminal
//! advertises 256 colors, secondary text uses a fixed gray from the 256-color
//! palette instead of the SGR "faint" attribute, which many terminals ignore.
//! Unicode status glyphs are used only when explicitly enabled and otherwise
//! fall back to ASCII.

use super::diagnostic::Level;

/// A styling palette. With `enabled = false` every method returns its input
/// unchanged, so rendered output contains no escape sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub enabled: bool,
    /// Terminal supports the 256-color palette (affects [`Style::dim`]).
    pub color_256: bool,
    /// Terminal can display Unicode glyphs.
    pub unicode: bool,
}

const RESET: &str = "\x1b[0m";

impl Style {
    pub const fn new(enabled: bool) -> Self {
        Self {
            enabled,
            color_256: false,
            unicode: false,
        }
    }

    pub const fn plain() -> Self {
        Self::new(false)
    }

    /// Enables the 256-color palette for secondary text.
    pub const fn with_256_colors(mut self, color_256: bool) -> Self {
        self.color_256 = color_256;
        self
    }

    /// Enables Unicode glyphs (independent of whether color is enabled).
    pub const fn with_unicode(mut self, unicode: bool) -> Self {
        self.unicode = unicode;
        self
    }

    /// Status glyph for a diagnostic level; `None` in ASCII mode so plain
    /// output stays exactly `error: ...`.
    pub fn glyph(&self, level: Level) -> Option<&'static str> {
        if !self.unicode {
            return None;
        }
        Some(match level {
            Level::Error => "\u{2716}",
            Level::Warning => "\u{26a0}",
            Level::Note => "\u{25cf}",
            Level::Info => "\u{2139}",
            Level::Success => "\u{2714}",
        })
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.enabled && !text.is_empty() {
            format!("\x1b[{code}m{text}{RESET}")
        } else {
            text.to_string()
        }
    }

    pub fn error(&self, text: &str) -> String {
        self.paint("1;31", text)
    }
    pub fn warning(&self, text: &str) -> String {
        self.paint("1;33", text)
    }
    pub fn note(&self, text: &str) -> String {
        self.paint("1;36", text)
    }
    pub fn info(&self, text: &str) -> String {
        self.paint("1;34", text)
    }
    pub fn success(&self, text: &str) -> String {
        self.paint("1;32", text)
    }
    pub fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }
    pub fn dim(&self, text: &str) -> String {
        if self.color_256 {
            self.paint("38;5;244", text)
        } else {
            self.paint("2", text)
        }
    }
}

/// Removes ANSI SGR escape sequences (`ESC [ … m`). Used by tests and when
/// measuring visible width.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_style_is_plain_text() {
        let style = Style::plain();
        assert_eq!(style.error("error"), "error");
        assert_eq!(style.dim("x"), "x");
    }

    #[test]
    fn enabled_style_wraps_and_strips() {
        let style = Style::new(true);
        let painted = style.success("Compiling");
        assert!(painted.starts_with("\x1b[1;32m") && painted.ends_with("\x1b[0m"));
        assert_eq!(strip_ansi(&painted), "Compiling");
    }

    #[test]
    fn dim_uses_256_gray_only_when_supported() {
        assert_eq!(Style::new(true).dim("x"), "\x1b[2mx\x1b[0m");
        assert_eq!(
            Style::new(true).with_256_colors(true).dim("x"),
            "\x1b[38;5;244mx\x1b[0m"
        );
        // Never any escape when color is off, even if 256 colors are known.
        assert_eq!(Style::plain().with_256_colors(true).dim("x"), "x");
    }

    #[test]
    fn glyphs_only_in_unicode_mode() {
        assert_eq!(Style::plain().glyph(Level::Error), None);
        let unicode = Style::plain().with_unicode(true);
        assert_eq!(unicode.glyph(Level::Error), Some("\u{2716}"));
        assert_eq!(unicode.glyph(Level::Success), Some("\u{2714}"));
    }
}
