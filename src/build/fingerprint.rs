//! Deterministic build fingerprint generation.

use super::BuildError;
use super::metadata::BuildConfiguration;
use super::toolchain::ToolchainInfo;
use crate::lock::Checksum;
use crate::package::PackageId;
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
