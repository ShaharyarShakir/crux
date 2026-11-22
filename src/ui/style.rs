//! ANSI styling that collapses to plain text when color is disabled.

/// A styling palette. With `enabled = false` every method returns its input
/// unchanged, so rendered output contains no escape sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub enabled: bool,
}

const RESET: &str = "\x1b[0m";

impl Style {
    pub const fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    pub const fn plain() -> Self {
        Self { enabled: false }
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
        self.paint("2", text)
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
}
