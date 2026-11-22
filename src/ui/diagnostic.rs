//! Structured diagnostics: data (`Diagnostic`) kept separate from rendering
//! (`Diagnostic::render`), so other renderers (for example JSON for IDEs) can
//! be added later without touching the code that produces diagnostics.

use super::style::Style;

/// Diagnostic severity / category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warning,
    Note,
    Info,
    Success,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Note => "note",
            Level::Info => "info",
            Level::Success => "success",
        }
    }

    fn paint(self, style: &Style, text: &str) -> String {
        match self {
            Level::Error => style.error(text),
            Level::Warning => style.warning(text),
            Level::Note => style.note(text),
            Level::Info => style.info(text),
            Level::Success => style.success(text),
        }
    }
}

/// A structured, renderer-independent diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: Level,
    /// Short, lowercase description (`package compilation failed`).
    pub title: String,
    /// Labelled facts such as `package`, `stage`, `exit status`, `build dir`.
    pub fields: Vec<(String, String)>,
    /// Explanatory paragraphs. Only facts Crux knows; never guessed causes.
    pub message: Vec<String>,
    /// The underlying command, when one was executed.
    pub command: Option<String>,
    /// Additional context (`note: …`).
    pub notes: Vec<String>,
    /// Actionable suggestions (`help: …`).
    pub help: Vec<String>,
}

impl Diagnostic {
    pub fn new(level: Level, title: impl Into<String>) -> Self {
        Self {
            level,
            title: title.into(),
            fields: Vec::new(),
            message: Vec::new(),
            command: None,
            notes: Vec::new(),
            help: Vec::new(),
        }
    }

    pub fn error(title: impl Into<String>) -> Self {
        Self::new(Level::Error, title)
    }

    pub fn warning(title: impl Into<String>) -> Self {
        Self::new(Level::Warning, title)
    }

    pub fn field(mut self, label: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.push((label.into(), value.into()));
        self
    }

    pub fn message(mut self, text: impl Into<String>) -> Self {
        self.message.push(text.into());
        self
    }

    pub fn command(mut self, command: impl Into<String>) -> Self {
        self.command = Some(command.into());
        self
    }

    pub fn note(mut self, text: impl Into<String>) -> Self {
        self.notes.push(text.into());
        self
    }

    pub fn help(mut self, text: impl Into<String>) -> Self {
        self.help.push(text.into());
        self
    }

    /// Renders the diagnostic as human-readable text ending in a newline.
    ///
    /// With a disabled [`Style`] the result contains no escape sequences.
    pub fn render(&self, style: &Style) -> String {
        let mut out = format!(
            "{}{} {}\n",
            self.level.paint(style, self.level.label()),
            style.bold(":"),
            style.bold(&self.title)
        );

        if !self.fields.is_empty() {
            out.push('\n');
            let width = self.fields.iter().map(|(l, _)| l.len()).max().unwrap_or(0);
            for (label, value) in &self.fields {
                let pad = " ".repeat(width - label.len());
                out.push_str(&format!(
                    "  {}{pad} {value}\n",
                    style.bold(&format!("{label}:"))
                ));
            }
        }
        for paragraph in &self.message {
            out.push('\n');
            for line in paragraph.lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }
        if let Some(command) = &self.command {
            out.push('\n');
            out.push_str(&format!("  {}\n", style.bold("command:")));
            for line in command.lines() {
                out.push_str(&format!("    {}\n", style.dim(line)));
            }
        }
        if !self.notes.is_empty() || !self.help.is_empty() {
            out.push('\n');
        }
        for note in &self.notes {
            out.push_str(&format!("  {} {note}\n", style.note("note:")));
        }
        for help in &self.help {
            out.push_str(&format!("  {} {help}\n", style.success("help:")));
        }
        out
    }
}

/// Formats a command line for display (see
/// [`crate::core::process::format_command_line`]).
pub fn display_command<I, S>(program: S, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    crate::core::process::format_command_line(program, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style::strip_ansi;

    fn sample() -> Diagnostic {
        Diagnostic::error("package compilation failed")
            .field("package", "fmt v11.1.1")
            .field("stage", "CMake build (compile and link)")
            .field("exit status", "1")
            .message("The build tool exited with status 1.")
            .command("cmake --build /tmp/b --config Debug")
            .note("original compiler output follows")
            .help("inspect the build directory: /tmp/b")
    }

    #[test]
    fn renders_plain_layout_exactly() {
        let text = sample().render(&Style::plain());
        assert_eq!(
            text,
            "error: package compilation failed\n\
             \n  package:     fmt v11.1.1\
             \n  stage:       CMake build (compile and link)\
             \n  exit status: 1\
             \n\n  The build tool exited with status 1.\
             \n\n  command:\
             \n    cmake --build /tmp/b --config Debug\
             \n\n  note: original compiler output follows\
             \n  help: inspect the build directory: /tmp/b\n"
        );
    }

    #[test]
    fn colored_rendering_differs_only_by_escape_sequences() {
        let plain = sample().render(&Style::plain());
        let colored = sample().render(&Style::new(true));
        assert_ne!(plain, colored);
        assert!(colored.contains("\x1b[1;31merror"));
        assert_eq!(strip_ansi(&colored), plain);
    }

    #[test]
    fn minimal_diagnostic_is_one_line() {
        let text = Diagnostic::new(Level::Warning, "registry is stale").render(&Style::plain());
        assert_eq!(text, "warning: registry is stale\n");
    }

    #[test]
    fn display_command_quotes_only_when_needed() {
        assert_eq!(
            display_command("cmake", ["--build", "/tmp/my build", "-DX=it's"]),
            "cmake --build '/tmp/my build' '-DX=it'\\''s'"
        );
    }
}
