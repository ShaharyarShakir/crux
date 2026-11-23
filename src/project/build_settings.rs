//! `[build]` manifest section: custom compiler flags, linker flags, and
//! sanitizers for the *application* being built.
//!
//! These settings never apply to dependency builds. Dependencies are built
//! and fingerprinted independently from their own recipes, so changing a
//! project's warning flags or sanitizers can neither break a dependency nor
//! silently invalidate its installation.
//!
//! The model is plain data: it is parsed and validated here, expanded into
//! compiler arguments by `build::sanitizer`, and emitted into the generated
//! CMake project one argument at a time (never through a shell).

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fmt;

/// Raw `[build]` table as written in `crux.toml`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawBuildSettings {
    #[serde(default)]
    c_flags: Option<Vec<String>>,
    #[serde(default)]
    cxx_flags: Option<Vec<String>>,
    #[serde(default)]
    linker_flags: Option<Vec<String>>,
    #[serde(default)]
    sanitizers: Option<RawSanitizers>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSanitizers {
    #[serde(default)]
    address: Option<bool>,
    #[serde(default)]
    undefined: Option<bool>,
}

/// A sanitizer Crux can enable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sanitizer {
    /// AddressSanitizer (`-fsanitize=address`).
    Address,
    /// UndefinedBehaviorSanitizer (`-fsanitize=undefined`).
    Undefined,
}

impl Sanitizer {
    /// The manifest key and `-fsanitize=` value.
    pub fn name(self) -> &'static str {
        match self {
            Sanitizer::Address => "address",
            Sanitizer::Undefined => "undefined",
        }
    }
}

impl fmt::Display for Sanitizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Sanitizer::Address => "AddressSanitizer",
            Sanitizer::Undefined => "UndefinedBehaviorSanitizer",
        })
    }
}

/// Validated `[build]` settings. The default is "nothing customised".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildSettings {
    /// Extra C compiler arguments, in declared order.
    pub c_flags: Vec<String>,
    /// Extra C++ compiler arguments, in declared order.
    pub cxx_flags: Vec<String>,
    /// Extra linker arguments, in declared order.
    pub linker_flags: Vec<String>,
    /// Enable AddressSanitizer.
    pub address_sanitizer: bool,
    /// Enable UndefinedBehaviorSanitizer.
    pub undefined_sanitizer: bool,
}

/// An invalid `[build]` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildSettingsError {
    pub key: String,
    pub error: String,
}

impl fmt::Display for BuildSettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid [build] setting '{}': {}", self.key, self.error)
    }
}

impl std::error::Error for BuildSettingsError {}

fn validate_flags(key: &str, flags: &[String]) -> Result<(), BuildSettingsError> {
    for (index, flag) in flags.iter().enumerate() {
        let fail = |error: String| BuildSettingsError {
            key: key.to_string(),
            error: format!("entry {} ({flag:?}) {error}", index + 1),
        };
        if flag.trim().is_empty() {
            return Err(fail("is empty".to_string()));
        }
        if flag != flag.trim() {
            return Err(fail(
                "has leading or trailing whitespace; each entry is one argument, \
                 so split '-I dir' into '-I', 'dir' or write '-Idir'"
                    .to_string(),
            ));
        }
        if flag.chars().any(char::is_control) {
            return Err(fail("contains a control character".to_string()));
        }
    }
    Ok(())
}

impl BuildSettings {
    /// Validates a parsed `[build]` table.
    pub(crate) fn from_raw(raw: RawBuildSettings) -> Result<Self, BuildSettingsError> {
        let settings = Self {
            c_flags: raw.c_flags.unwrap_or_default(),
            cxx_flags: raw.cxx_flags.unwrap_or_default(),
            linker_flags: raw.linker_flags.unwrap_or_default(),
            address_sanitizer: raw
                .sanitizers
                .as_ref()
                .and_then(|s| s.address)
                .unwrap_or(false),
            undefined_sanitizer: raw
                .sanitizers
                .as_ref()
                .and_then(|s| s.undefined)
                .unwrap_or(false),
        };
        settings.validate()?;
        Ok(settings)
    }

    /// Validates every flag list.
    pub fn validate(&self) -> Result<(), BuildSettingsError> {
        validate_flags("c_flags", &self.c_flags)?;
        validate_flags("cxx_flags", &self.cxx_flags)?;
        validate_flags("linker_flags", &self.linker_flags)
    }

    /// True when nothing is customised (the section can be omitted).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Enabled sanitizers in canonical order.
    pub fn sanitizers(&self) -> Vec<Sanitizer> {
        let mut list = Vec::new();
        if self.address_sanitizer {
            list.push(Sanitizer::Address);
        }
        if self.undefined_sanitizer {
            list.push(Sanitizer::Undefined);
        }
        list
    }

    /// Canonical, unambiguous byte encoding used for fingerprints.
    ///
    /// Flag lists keep their declared order (later compiler flags override
    /// earlier ones, so order is semantic). Entries are length-prefixed so
    /// `["a b"]` and `["a", "b"]` can never collide. Sanitizers are encoded
    /// in a fixed order independent of how they were written.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (label, flags) in [
            ("c_flags", &self.c_flags),
            ("cxx_flags", &self.cxx_flags),
            ("linker_flags", &self.linker_flags),
        ] {
            out.extend_from_slice(format!("{label}:{}\n", flags.len()).as_bytes());
            for flag in flags {
                out.extend_from_slice(format!("{}:", flag.len()).as_bytes());
                out.extend_from_slice(flag.as_bytes());
                out.push(b'\n');
            }
        }
        for sanitizer in [Sanitizer::Address, Sanitizer::Undefined] {
            let on = self.sanitizers().contains(&sanitizer);
            out.extend_from_slice(
                format!("sanitizer.{}:{}\n", sanitizer.name(), on as u8).as_bytes(),
            );
        }
        out
    }

    /// SHA-256 over [`BuildSettings::canonical_bytes`], as lowercase hex.
    pub fn digest(&self) -> String {
        format!("{:x}", Sha256::digest(self.canonical_bytes()))
    }

    /// Renders the `[build]` section (empty string when default).
    pub fn to_toml_section(&self) -> String {
        if self.is_default() {
            return String::new();
        }
        let array = |flags: &[String]| {
            let items: Vec<String> = flags
                .iter()
                .map(|f| toml::Value::String(f.clone()).to_string())
                .collect();
            format!("[{}]", items.join(", "))
        };
        let mut out = String::from("\n[build]\n");
        if !self.c_flags.is_empty() {
            out.push_str(&format!("c_flags = {}\n", array(&self.c_flags)));
        }
        if !self.cxx_flags.is_empty() {
            out.push_str(&format!("cxx_flags = {}\n", array(&self.cxx_flags)));
        }
        if !self.linker_flags.is_empty() {
            out.push_str(&format!("linker_flags = {}\n", array(&self.linker_flags)));
        }
        if self.address_sanitizer || self.undefined_sanitizer {
            out.push_str("\n[build.sanitizers]\n");
            if self.address_sanitizer {
                out.push_str("address = true\n");
            }
            if self.undefined_sanitizer {
                out.push_str("undefined = true\n");
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_is_empty_and_renders_nothing() {
        let settings = BuildSettings::default();
        assert!(settings.is_default());
        assert_eq!(settings.to_toml_section(), "");
    }

    #[test]
    fn rejects_empty_padded_and_control_flags() {
        for bad in ["", "   ", " -Wall", "-Wall ", "-W\nall", "-W\0all"] {
            let settings = BuildSettings {
                c_flags: vec![bad.to_string()],
                ..Default::default()
            };
            let err = settings.validate().unwrap_err();
            assert_eq!(err.key, "c_flags", "{bad:?}");
        }
    }

    #[test]
    fn internal_spaces_are_one_argument() {
        let settings = BuildSettings {
            cxx_flags: flags(&["-DGREETING=hello world"]),
            ..Default::default()
        };
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn digest_is_order_and_boundary_sensitive() {
        let a = BuildSettings {
            c_flags: flags(&["-O2", "-O3"]),
            ..Default::default()
        };
        let b = BuildSettings {
            c_flags: flags(&["-O3", "-O2"]),
            ..Default::default()
        };
        assert_ne!(a.digest(), b.digest());

        let joined = BuildSettings {
            c_flags: flags(&["-I a"]),
            ..Default::default()
        };
        let split = BuildSettings {
            c_flags: flags(&["-I", "a"]),
            ..Default::default()
        };
        assert_ne!(joined.digest(), split.digest());
        assert_eq!(a.digest(), a.clone().digest());
    }

    #[test]
    fn digest_distinguishes_every_field() {
        let base = BuildSettings::default();
        let variants = [
            BuildSettings {
                c_flags: flags(&["-Wall"]),
                ..Default::default()
            },
            BuildSettings {
                cxx_flags: flags(&["-Wall"]),
                ..Default::default()
            },
            BuildSettings {
                linker_flags: flags(&["-Wall"]),
                ..Default::default()
            },
            BuildSettings {
                address_sanitizer: true,
                ..Default::default()
            },
            BuildSettings {
                undefined_sanitizer: true,
                ..Default::default()
            },
        ];
        let mut seen = std::collections::BTreeSet::new();
        seen.insert(base.digest());
        for variant in &variants {
            assert!(
                seen.insert(variant.digest()),
                "digest collision: {variant:?}"
            );
        }
    }

    #[test]
    fn toml_section_round_trips_through_the_manifest() {
        let settings = BuildSettings {
            c_flags: flags(&["-Wall", "-DNAME=\"x y\""]),
            cxx_flags: flags(&["-Wextra"]),
            linker_flags: flags(&["-Wl,--as-needed"]),
            address_sanitizer: true,
            undefined_sanitizer: true,
        };
        let text = format!(
            "[project]\nname = \"demo\"\nversion = \"0.1.0\"\nlanguage = \"cpp\"\n{}",
            settings.to_toml_section()
        );
        let manifest = crate::project::Manifest::from_toml_str(&text).unwrap();
        assert_eq!(manifest.build, settings);
    }
}
