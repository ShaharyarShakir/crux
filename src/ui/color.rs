//! Color policy: `--color`, `CLICOLOR_FORCE`, `NO_COLOR`, and terminal detection.
//!
//! Precedence, highest first:
//!
//! 1. `--color always` / `--color never` on the command line.
//! 2. `CLICOLOR_FORCE` set to a non-empty value other than `0` → color on.
//! 3. `NO_COLOR` set to any non-empty value → color off.
//! 4. `--color auto` (the default): color only when stderr is a terminal and
//!    `TERM` is not `dumb`.
//!
//! An explicit command-line choice always wins over the environment, so a
//! user can override a CI-wide `NO_COLOR` or `CLICOLOR_FORCE` for one run.

use std::fmt;
use std::str::FromStr;

/// The `--color` command-line setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

impl FromStr for ColorChoice {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            other => Err(format!(
                "invalid value '{other}' for '--color': expected 'auto', 'always', or 'never'"
            )),
        }
    }
}

impl fmt::Display for ColorChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        })
    }
}

/// The color-related environment, captured once so the policy is testable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColorEnv {
    pub no_color: Option<String>,
    pub clicolor_force: Option<String>,
    pub term: Option<String>,
}

impl ColorEnv {
    /// Reads `NO_COLOR`, `CLICOLOR_FORCE`, and `TERM` from the process environment.
    pub fn from_process() -> Self {
        Self {
            no_color: std::env::var("NO_COLOR").ok(),
            clicolor_force: std::env::var("CLICOLOR_FORCE").ok(),
            term: std::env::var("TERM").ok(),
        }
    }
}

/// Decides whether ANSI styling should be emitted.
pub fn should_colorize(choice: ColorChoice, env: &ColorEnv, stderr_is_terminal: bool) -> bool {
    match choice {
        ColorChoice::Always => return true,
        ColorChoice::Never => return false,
        ColorChoice::Auto => {}
    }
    if env
        .clicolor_force
        .as_deref()
        .is_some_and(|v| !v.is_empty() && v != "0")
    {
        return true;
    }
    if env.no_color.as_deref().is_some_and(|v| !v.is_empty()) {
        return false;
    }
    stderr_is_terminal && env.term.as_deref() != Some("dumb")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(no_color: Option<&str>, force: Option<&str>, term: Option<&str>) -> ColorEnv {
        ColorEnv {
            no_color: no_color.map(str::to_string),
            clicolor_force: force.map(str::to_string),
            term: term.map(str::to_string),
        }
    }

    #[test]
    fn explicit_choice_wins_over_environment() {
        let forced = env(None, Some("1"), None);
        let disabled = env(Some("1"), None, None);
        assert!(!should_colorize(ColorChoice::Never, &forced, true));
        assert!(should_colorize(ColorChoice::Always, &disabled, false));
    }

    #[test]
    fn auto_follows_terminal_and_term() {
        let plain = env(None, None, Some("xterm-256color"));
        assert!(should_colorize(ColorChoice::Auto, &plain, true));
        assert!(
            !should_colorize(ColorChoice::Auto, &plain, false),
            "piped output"
        );
        let dumb = env(None, None, Some("dumb"));
        assert!(!should_colorize(ColorChoice::Auto, &dumb, true));
    }

    #[test]
    fn no_color_and_clicolor_force_precedence() {
        assert!(!should_colorize(
            ColorChoice::Auto,
            &env(Some("1"), None, None),
            true
        ));
        // Empty NO_COLOR is ignored, per the no-color.org convention.
        assert!(should_colorize(
            ColorChoice::Auto,
            &env(Some(""), None, None),
            true
        ));
        assert!(should_colorize(
            ColorChoice::Auto,
            &env(None, Some("1"), None),
            false
        ));
        assert!(!should_colorize(
            ColorChoice::Auto,
            &env(None, Some("0"), None),
            false
        ));
        // CLICOLOR_FORCE outranks NO_COLOR when both are set.
        assert!(should_colorize(
            ColorChoice::Auto,
            &env(Some("1"), Some("1"), None),
            false
        ));
    }

    #[test]
    fn parses_cli_values() {
        assert_eq!("always".parse::<ColorChoice>(), Ok(ColorChoice::Always));
        assert!("sometimes".parse::<ColorChoice>().is_err());
    }
}
