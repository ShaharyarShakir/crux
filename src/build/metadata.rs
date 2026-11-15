//! Build profiles, artifact classification, configuration, and metadata models.

use super::BuildError;
use super::fingerprint::BuildFingerprint;
use super::toolchain::ToolchainInfo;
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Current format version for build metadata files.
pub const CURRENT_BUILD_METADATA_VERSION: u32 = 1;

/// Supported build profiles for CMake compilation.
#[derive(
    Default,
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
)]
pub enum BuildProfile {
    #[serde(rename = "debug")]
    Debug,
    #[serde(rename = "release")]
    #[default]
    Release,
}

impl BuildProfile {
    /// Returns the standard CMake `CMAKE_BUILD_TYPE` string.
    pub fn as_cmake_build_type(&self) -> &'static str {
        match self {
            Self::Debug => "Debug",
            Self::Release => "Release",
        }
    }

    /// Returns the canonical lowercase representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

impl fmt::Display for BuildProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for BuildProfile {
    type Err = BuildError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "debug" => Ok(Self::Debug),
            "release" => Ok(Self::Release),
            other => Err(BuildError::InvalidConfiguration(format!(
                "invalid build profile '{other}': expected 'debug' or 'release'"
            ))),
        }
    }
}

/// Classification of artifacts produced by a C/C++ build.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum ArtifactType {
    StaticLibrary,
    SharedLibrary,
    Executable,
    HeaderOnly,
    Other,
}

impl ArtifactType {
    /// Classifies an artifact from its filesystem path.
    pub fn from_path(path: &Path) -> Self {
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        if file_name.ends_with(".a") || file_name.ends_with(".lib") {
            return Self::StaticLibrary;
        }

        if file_name.ends_with(".so")
            || file_name.contains(".so.")
            || file_name.ends_with(".dylib")
            || file_name.ends_with(".dll")
        {
            return Self::SharedLibrary;
        }

        if file_name.ends_with(".h")
            || file_name.ends_with(".hpp")
            || file_name.ends_with(".hxx")
            || file_name.ends_with(".hh")
        {
            return Self::HeaderOnly;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = fs::metadata(path)
                && meta.is_file()
                && (meta.permissions().mode() & 0o111 != 0)
            {
                // Check if file is in bin/ or has no standard non-executable extension
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if ext.is_empty()
                    || ext == "exe"
                    || path.components().any(|c| c.as_os_str() == "bin")
                {
                    return Self::Executable;
                }
            }
        }

        #[cfg(windows)]
        {
            if file_name.ends_with(".exe") {
                return Self::Executable;
            }
        }

        Self::Other
    }

    /// Returns human-readable label.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::StaticLibrary => "static library",
            Self::SharedLibrary => "shared library",
            Self::Executable => "executable",
            Self::HeaderOnly => "header",
            Self::Other => "other artifact",
        }
    }
}

impl fmt::Display for ArtifactType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A specific file artifact produced during compilation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct BuildArtifact {
    pub name: String,
    pub relative_path: PathBuf,
    pub path: PathBuf,
    pub artifact_type: ArtifactType,
    pub size_bytes: u64,
}

/// Deterministic build configuration parameters.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BuildConfiguration {
    pub profile: BuildProfile,
    #[serde(default)]
    pub generator: Option<String>,
    #[serde(default)]
    pub cmake_args: Vec<String>,
    #[serde(default)]
    pub c_compiler: Option<PathBuf>,
    #[serde(default)]
    pub cxx_compiler: Option<PathBuf>,
    #[serde(default)]
    pub jobs: Option<usize>,
    #[serde(default)]
    pub build_shared_libs: Option<bool>,
    #[serde(default)]
    pub extra_env: BTreeMap<String, String>,
}

impl Default for BuildConfiguration {
    fn default() -> Self {
        Self {
            profile: BuildProfile::Release,
            generator: None,
            cmake_args: Vec::new(),
            c_compiler: None,
            cxx_compiler: None,
            jobs: None,
            build_shared_libs: None,
            extra_env: BTreeMap::new(),
        }
    }
}

impl BuildConfiguration {
    /// Creates a new configuration with the specified build profile.
    pub fn with_profile(profile: BuildProfile) -> Self {
        Self {
            profile,
            ..Default::default()
        }
    }
}

/// Persistent metadata recorded upon successful compilation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BuildMetadata {
    pub format_version: u32,
    pub package_name: String,
    pub package_version: String,
    pub source_checksum: String,
    pub fingerprint: BuildFingerprint,
    pub profile: BuildProfile,
    pub toolchain: ToolchainInfo,
    pub configuration: BuildConfiguration,
    pub source_dir: PathBuf,
    pub build_dir: PathBuf,
    pub configure_exit_code: i32,
    pub build_exit_code: i32,
    pub duration_millis: u64,
    pub artifacts: Vec<BuildArtifact>,
    pub success: bool,
}
