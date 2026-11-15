//! Installation metadata, file classification, and manifest models.

use crate::build::{BuildFingerprint, BuildProfile};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

/// Current format version for installation metadata files.
pub const CURRENT_INSTALL_METADATA_VERSION: u32 = 1;

/// Standard filename for installation metadata.
pub const INSTALL_METADATA_FILENAME: &str = "installed.toml";

/// Classification of installed package files.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum InstalledFileType {
    #[serde(rename = "header")]
    Header,
    #[serde(rename = "static_lib")]
    StaticLibrary,
    #[serde(rename = "shared_lib")]
    SharedLibrary,
    #[serde(rename = "executable")]
    Executable,
    #[serde(rename = "pkgconfig")]
    PkgConfig,
    #[serde(rename = "cmake_module")]
    CMakeModule,
    #[serde(rename = "other")]
    Other,
}

impl InstalledFileType {
    /// Classifies an installed file given its relative path from the install root and full path.
    pub fn classify(rel_path: &Path, full_path: &Path) -> Self {
        let file_name = rel_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        // 1. Pkg-config files
        if file_name.ends_with(".pc") {
            return Self::PkgConfig;
        }

        // 2. CMake configuration / module files
        if file_name.ends_with(".cmake") || rel_path.components().any(|c| c.as_os_str() == "cmake")
        {
            return Self::CMakeModule;
        }

        // 3. Headers
        if file_name.ends_with(".h")
            || file_name.ends_with(".hpp")
            || file_name.ends_with(".hxx")
            || file_name.ends_with(".hh")
            || file_name.ends_with(".inc")
            || file_name.ends_with(".inl")
            || rel_path.components().any(|c| c.as_os_str() == "include")
        {
            return Self::Header;
        }

        // 4. Static libraries
        if file_name.ends_with(".a") || file_name.ends_with(".lib") {
            return Self::StaticLibrary;
        }

        // 5. Shared libraries
        if file_name.ends_with(".so")
            || file_name.contains(".so.")
            || file_name.ends_with(".dylib")
            || file_name.ends_with(".dll")
        {
            return Self::SharedLibrary;
        }

        // 6. Executables
        if rel_path.components().any(|c| c.as_os_str() == "bin") || file_name.ends_with(".exe") {
            return Self::Executable;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = fs::symlink_metadata(full_path)
                && meta.is_file()
                && (meta.permissions().mode() & 0o111 != 0)
            {
                return Self::Executable;
            }
        }

        Self::Other
    }

    /// Returns human-readable label.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::StaticLibrary => "static library",
            Self::SharedLibrary => "shared library",
            Self::Executable => "executable",
            Self::PkgConfig => "pkg-config",
            Self::CMakeModule => "CMake module",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for InstalledFileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// An individual file tracked in the installation inventory.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct InstalledFile {
    pub relative_path: PathBuf,
    pub file_type: InstalledFileType,
    pub size_bytes: u64,
}

/// Persistent installation metadata written to `installed.toml`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstallMetadata {
    pub format_version: u32,
    pub package_name: String,
    pub package_version: String,
    pub fingerprint: BuildFingerprint,
    pub profile: BuildProfile,
    pub installed_files: Vec<InstalledFile>,
    pub total_size_bytes: u64,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<String>,
}
