//! Writer adapters used by the CLI entry point.

use super::style::Style;
use std::io::{self, Write};

/// Paints a label with one of the [`Style`] methods.
type Painter = fn(&Style, &str) -> String;

/// Colors the leading `error:`, `warning:`, `note:`, and `help:` label of
/// each line written through it, so existing diagnostics gain consistent
/// styling without changing their text.
///
/// When styling is disabled this is a transparent pass-through: bytes are
/// forwarded unchanged and immediately. When enabled, output is forwarded
/// line by line; `flush` forwards any partial line.
pub struct StyledWriter<W: Write> {
    inner: W,
    style: Style,
    pending: Vec<u8>,
}

impl<W: Write> StyledWriter<W> {
    pub fn new(inner: W, style: Style) -> Self {
        Self {
            inner,
            style,
            pending: Vec::new(),
        }
    }

    fn style_line(&self, line: &[u8]) -> Vec<u8> {
        let labels: [(&str, Painter); 4] = [
            ("error:", Style::error),
            ("warning:", Style::warning),
            ("note:", Style::note),
            ("help:", Style::success),
        ];
        for (label, paint) in labels {
            if line.starts_with(label.as_bytes()) {
                let mut out = paint(&self.style, &label[..label.len() - 1]).into_bytes();
                out.extend_from_slice(&line[label.len() - 1..]);
                return out;
            }
        }
        line.to_vec()
    }

    fn forward_complete_lines(&mut self) -> io::Result<()> {
        while let Some(pos) = self.pending.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=pos).collect();
            let styled = self.style_line(&line);
            self.inner.write_all(&styled)?;
        }
        Ok(())
    }
}

impl<W: Write> Write for StyledWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.style.enabled {
            return self.inner.write(buf);
        }
        self.pending.extend_from_slice(buf);
        self.forward_complete_lines()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            let styled = self.style_line(&rest);
            self.inner.write_all(&styled)?;
        }
        self.inner.flush()
    }
}

impl<W: Write> Drop for StyledWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

/// Treats a closed pipe (for example `crux resolve | head -1`) as the end of
/// interest in output: the first `BrokenPipe` is swallowed and all further
/// output is discarded, instead of turning into an error that changes the
/// command's behavior or exit status.
pub struct PipeTolerantWriter<W: Write> {
    inner: W,
    closed: bool,
}

impl<W: Write> PipeTolerantWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            closed: false,
        }
    }

    pub fn pipe_closed(&self) -> bool {
        self.closed
    }
}

impl<W: Write> Write for PipeTolerantWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.closed {
            return Ok(buf.len());
        }
        match self.inner.write(buf) {
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => {
                self.closed = true;
                Ok(buf.len())
            }
            other => other,
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.closed {
            return Ok(());
        }
        match self.inner.flush() {
            Err(err) if err.kind() == io::ErrorKind::BrokenPipe => {
                self.closed = true;
                Ok(())
            }
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::style::strip_ansi;

    #[test]
    fn disabled_styling_is_byte_identical() {
        let mut out = Vec::new();
        {
            let mut w = StyledWriter::new(&mut out, Style::plain());
            write!(w, "error: bad\npartial").unwrap();
        }
        assert_eq!(out, b"error: bad\npartial");
    }

    #[test]
    fn enabled_styling_colors_only_leading_labels() {
        let mut out = Vec::new();
        {
            let mut w = StyledWriter::new(&mut out, Style::new(true));
            write!(w, "warn").unwrap();
            write!(
                w,
                "ing: split write\nsrc/a.c:1:2: error: compiler line\nerror: x"
            )
            .unwrap();
        }
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.starts_with("\x1b[1;33mwarning\x1b[0m: split write\n"),
            "{text:?}"
        );
        assert!(
            text.contains("src/a.c:1:2: error: compiler line\n"),
            "{text:?}"
        );
        assert!(text.ends_with("\x1b[1;31merror\x1b[0m: x"), "{text:?}");
        assert_eq!(
            strip_ansi(&text),
            "warning: split write\nsrc/a.c:1:2: error: compiler line\nerror: x"
        );
    }

    struct ClosedPipe;
    impl Write for ClosedPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn broken_pipe_is_swallowed() {
        let mut w = PipeTolerantWriter::new(ClosedPipe);
        assert!(writeln!(w, "first").is_ok());
        assert!(writeln!(w, "second").is_ok());
        assert!(w.pipe_closed());
    }
}
