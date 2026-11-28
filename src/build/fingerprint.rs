//! Deterministic build fingerprint generation.

use super::BuildError;
use super::metadata::BuildConfiguration;
use super::toolchain::{CompilerInfo, ToolchainInfo};
use crate::lock::Checksum;
use crate::package::PackageId;
use crate::project::build_settings::BuildSettings;
use crate::project::manifest::ProjectLanguage;
use sha2::{Digest, Sha256};
use std::fmt;

/// Cryptographic SHA-256 fingerprint representing a package's exact build environment and inputs.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct BuildFingerprint(String);

impl BuildFingerprint {
    /// Constructs a build fingerprint from a formatted string (must start with "sha256:").
    pub fn new(raw: impl Into<String>) -> Result<Self, BuildError> {
        let s = raw.into();
        if !s.starts_with("sha256:") || s.len() != "sha256:".len() + 64 {
            return Err(BuildError::InvalidConfiguration(format!(
                "invalid build fingerprint format '{s}': expected sha256:<64 hex chars>"
            )));
        }
        let hex_part = &s["sha256:".len()..];
        if !hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(BuildError::InvalidConfiguration(format!(
                "invalid non-hex characters in build fingerprint '{s}'"
            )));
        }
        Ok(Self(s))
    }

    /// Computes a canonical, deterministic build fingerprint from all build inputs.
    pub fn compute(
        package_id: &PackageId,
        source_checksum: &Checksum,
        toolchain: &ToolchainInfo,
        config: &BuildConfiguration,
    ) -> Self {
        Self::compute_with_inputs(package_id, source_checksum, toolchain, config, &[], &[])
    }

    /// Like [`BuildFingerprint::compute`], also covering the resolved compilers
    /// and ambient flags, and the installed Crux dependencies. With neither,
    /// the digest is identical.
    pub(crate) fn compute_with_inputs(
        package_id: &PackageId,
        source_checksum: &Checksum,
        toolchain: &ToolchainInfo,
        config: &BuildConfiguration,
        environment: &[(&str, &[u8])],
        dependencies: &[(&str, &[u8])],
    ) -> Self {
        let mut hasher = Sha256::new();

        // 1. Package Identity
        hasher.update(package_id.name().as_str().as_bytes());
        hasher.update(b"@");
        hasher.update(package_id.version().to_string().as_bytes());
        hasher.update(b"\n");

        // 2. Source Digest
        hasher.update(source_checksum.digest().as_bytes());
        hasher.update(b"\n");

        // 3. Profile
        hasher.update(config.profile.as_str().as_bytes());
        hasher.update(b"\n");

        // 4. Toolchain details
        hasher.update(toolchain.cmake_version.as_bytes());
        hasher.update(b"\n");
        hasher.update(toolchain.generator.as_bytes());
        hasher.update(b"\n");
        if let Some(v) = &toolchain.c_compiler_version {
            hasher.update(v.as_bytes());
        }
        hasher.update(b"\n");
        if let Some(v) = &toolchain.cxx_compiler_version {
            hasher.update(v.as_bytes());
        }
        hasher.update(b"\n");
        if let Some(p) = &config.c_compiler {
            hasher.update(p.to_string_lossy().as_bytes());
        }
        hasher.update(b"\n");
        if let Some(p) = &config.cxx_compiler {
            hasher.update(p.to_string_lossy().as_bytes());
        }
        hasher.update(b"\n");

        // 5. Shared libs flag
        if let Some(shared) = config.build_shared_libs {
            hasher.update(if shared { b"shared:1\n" } else { b"shared:0\n" });
        }

        // 6. Sorted CMake args
        let mut sorted_args = config.cmake_args.clone();
        sorted_args.sort();
        for arg in &sorted_args {
            hasher.update(arg.as_bytes());
            hasher.update(b"\0");
        }

        // 7. Sorted extra environment variables
        for (k, v) in &config.extra_env {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(v.as_bytes());
            hasher.update(b"\0");
        }

        // 8. Non-default compilers and ambient flags, then
        // 9. dependency installations. Length-prefixed, and only when present.
        for (header, fields) in [
            (&b"\x01crux-toolchain-env-v1\0"[..], environment),
            (&b"\x01crux-package-deps-v1\0"[..], dependencies),
        ] {
            if fields.is_empty() {
                continue;
            }
            hasher.update(header);
            for (label, value) in fields {
                hasher.update(label.as_bytes());
                hasher.update(b"=");
                hasher.update(value.len().to_string().as_bytes());
                hasher.update(b":");
                hasher.update(value);
                hasher.update(b"\n");
            }
        }

        let digest = format!("{:x}", hasher.finalize());
        Self(format!("sha256:{digest}"))
    }

    /// Returns the full fingerprint string (`sha256:<64 hex>`).
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the 64-hex character digest without the `sha256:` prefix.
    pub fn hex_digest(&self) -> &str {
        &self.0["sha256:".len()..]
    }
}

impl fmt::Display for BuildFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Everything that can change the compiled output of an application build.
#[derive(Debug, Clone, Copy)]
pub struct AppFingerprintInputs<'a> {
    pub compiler: &'a CompilerInfo,
    pub language: ProjectLanguage,
    pub standard: &'a str,
    pub profile: &'a str,
    pub cmake_version: &'a str,
    pub generator: Option<&'a str>,
    /// `[build]` flags and sanitizers.
    pub settings: &'a BuildSettings,
    /// Fingerprints of linked dependency installations, in link order.
    pub dependency_fingerprints: &'a [String],
}

/// Computes a deterministic fingerprint of an application build
/// configuration (`sha256:<hex>`). Used to discard a build tree that was
/// produced under a different configuration instead of reusing its objects.
///
/// Compiler identity is the family, exact version, target triple and path;
/// flags keep their declared order because later flags override earlier
/// ones. The format is versioned (`crux-app-build-v1`) so it can evolve
/// without colliding with older records.
pub fn app_build_fingerprint(inputs: &AppFingerprintInputs<'_>) -> String {
    let mut hasher = Sha256::new();
    let mut field = |label: &str, value: &[u8]| {
        hasher.update(label.as_bytes());
        hasher.update(b"=");
        hasher.update(value.len().to_string().as_bytes());
        hasher.update(b":");
        hasher.update(value);
        hasher.update(b"\n");
    };
    field("format", b"crux-app-build-v1");
    field(
        "compiler.family",
        inputs.compiler.family.to_string().as_bytes(),
    );
    field("compiler.version", inputs.compiler.version.as_bytes());
    field("compiler.target", inputs.compiler.target_triple.as_bytes());
    field(
        "compiler.path",
        inputs.compiler.path.to_string_lossy().as_bytes(),
    );
    field("language", inputs.language.to_string().as_bytes());
    field("standard", inputs.standard.as_bytes());
    field("profile", inputs.profile.as_bytes());
    field("cmake", inputs.cmake_version.as_bytes());
    field("generator", inputs.generator.unwrap_or("").as_bytes());
    field("settings", &inputs.settings.canonical_bytes());
    for dependency in inputs.dependency_fingerprints {
        field("dependency", dependency.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::toolchain::CompilerFamily;
    use std::path::PathBuf;

    fn compiler(version: &str) -> CompilerInfo {
        CompilerInfo {
            family: CompilerFamily::Gcc,
            language: ProjectLanguage::Cpp,
            version: version.to_string(),
            target_triple: "x86_64-linux-gnu".to_string(),
            path: PathBuf::from("/usr/bin/g++"),
        }
    }

    fn digest(
        cc: &CompilerInfo,
        settings: &BuildSettings,
        profile: &str,
        deps: &[String],
    ) -> String {
        app_build_fingerprint(&AppFingerprintInputs {
            compiler: cc,
            language: ProjectLanguage::Cpp,
            standard: "c++20",
            profile,
            cmake_version: "CMake 3.30.0",
            generator: Some("Ninja"),
            settings,
            dependency_fingerprints: deps,
        })
    }

    /// The M0 golden inputs and digest (`tests/m0_fingerprint_golden.rs`).
    const PACKAGE_GOLDEN: &str =
        "sha256:94fd6a1e03a50361cb9312ff7f9765fc197a568de1271b54f149345c10bb9cfe";

    fn package_golden_inputs() -> (PackageId, Checksum, ToolchainInfo, BuildConfiguration) {
        let id = PackageId::new(
            crate::package::PackageName::new("char-lib").unwrap(),
            semver::Version::new(1, 2, 3),
        );
        let checksum = Checksum::new_sha256("0123456789abcdef".repeat(4)).unwrap();
        let toolchain = ToolchainInfo {
            cmake_version: "cmake version 3.31.0".to_string(),
            c_compiler_version: Some("gcc (GCC) 14.2.0".to_string()),
            cxx_compiler_version: Some("g++ (GCC) 14.2.0".to_string()),
            generator: "Unix Makefiles".to_string(),
        };
        let config = BuildConfiguration {
            profile: crate::build::BuildProfile::Debug,
            generator: Some("Unix Makefiles".to_string()),
            cmake_args: vec!["-DSECOND=2".into(), "-DFIRST=1".into()],
            c_compiler: Some("/opt/toolchains/char-gcc/cc".into()),
            cxx_compiler: Some("/opt/toolchains/char-gcc/c++".into()),
            jobs: Some(8),
            build_shared_libs: Some(true),
            extra_env: [("CRUX_M0_A", "alpha"), ("CRUX_M0_B", "beta")]
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .into(),
        };
        (id, checksum, toolchain, config)
    }

    #[test]
    fn engine_path_without_extra_inputs_matches_the_package_golden() {
        let (id, checksum, toolchain, config) = package_golden_inputs();
        let digest =
            BuildFingerprint::compute_with_inputs(&id, &checksum, &toolchain, &config, &[], &[]);
        assert_eq!(digest.as_str(), PACKAGE_GOLDEN);
    }

    /// Computed outside Rust from the same byte stream plus the dependency
    /// section, so an accidental change of encoding is caught.
    #[test]
    fn dependency_section_matches_its_golden() {
        let (id, checksum, toolchain, config) = package_golden_inputs();
        let (leaf, zlib) = (
            format!("sha256:{}", "a".repeat(64)),
            format!("sha256:{}", "b".repeat(64)),
        );
        let fields: [(&str, &[u8]); 6] = [
            ("name", b"leaf"),
            ("version", b"1.0.0"),
            ("fingerprint", leaf.as_bytes()),
            ("name", b"zlib"),
            ("version", b"1.3.1"),
            ("fingerprint", zlib.as_bytes()),
        ];
        let digest = BuildFingerprint::compute_with_inputs(
            &id,
            &checksum,
            &toolchain,
            &config,
            &[],
            &fields,
        );
        assert_eq!(
            digest.as_str(),
            "sha256:704ebcce9cbf177821cfd5f23ecbbaba93714b2bbab896e6059327c772f78ed1"
        );
    }

    #[test]
    fn fingerprint_is_deterministic_and_well_formed() {
        let cc = compiler("13.2.0");
        let s = BuildSettings::default();
        let a = digest(&cc, &s, "debug", &[]);
        assert_eq!(a, digest(&cc, &s, "debug", &[]));
        assert!(BuildFingerprint::new(a).is_ok());
    }

    #[test]
    fn every_output_affecting_input_changes_the_fingerprint() {
        let cc = compiler("13.2.0");
        let base = BuildSettings::default();
        let baseline = digest(&cc, &base, "debug", &[]);

        let mut variants: Vec<(&str, String)> = vec![
            (
                "compiler version",
                digest(&compiler("14.1.0"), &base, "debug", &[]),
            ),
            ("profile", digest(&cc, &base, "release", &[])),
            (
                "dependency",
                digest(&cc, &base, "debug", &["abc".to_string()]),
            ),
        ];
        let tweaks: [(&str, BuildSettings); 5] = [
            (
                "c_flags",
                BuildSettings {
                    c_flags: vec!["-Wall".into()],
                    ..Default::default()
                },
            ),
            (
                "cxx_flags",
                BuildSettings {
                    cxx_flags: vec!["-Wall".into()],
                    ..Default::default()
                },
            ),
            (
                "linker_flags",
                BuildSettings {
                    linker_flags: vec!["-Wl,-z,now".into()],
                    ..Default::default()
                },
            ),
            (
                "asan",
                BuildSettings {
                    address_sanitizer: true,
                    ..Default::default()
                },
            ),
            (
                "ubsan",
                BuildSettings {
                    undefined_sanitizer: true,
                    ..Default::default()
                },
            ),
        ];
        for (name, settings) in &tweaks {
            variants.push((name, digest(&cc, settings, "debug", &[])));
        }
        let mut seen = std::collections::BTreeSet::from([baseline]);
        for (name, value) in variants {
            assert!(
                seen.insert(value),
                "changing {name} did not change the fingerprint"
            );
        }
    }

    #[test]
    fn flag_order_changes_the_fingerprint() {
        let cc = compiler("13.2.0");
        let forward = BuildSettings {
            cxx_flags: vec!["-O0".into(), "-O2".into()],
            ..Default::default()
        };
        let reverse = BuildSettings {
            cxx_flags: vec!["-O2".into(), "-O0".into()],
            ..Default::default()
        };
        assert_ne!(
            digest(&cc, &forward, "debug", &[]),
            digest(&cc, &reverse, "debug", &[])
        );
    }

    #[test]
    fn sanitizer_declaration_order_does_not_matter() {
        // address/undefined are booleans; the canonical encoding is fixed.
        let cc = compiler("13.2.0");
        let both = BuildSettings {
            address_sanitizer: true,
            undefined_sanitizer: true,
            ..Default::default()
        };
        assert_eq!(
            digest(&cc, &both, "debug", &[]),
            digest(&cc, &both.clone(), "debug", &[])
        );
    }
}
