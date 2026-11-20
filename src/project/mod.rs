//! Project management, manifest parsing, initialization, and verification.

pub mod check;
pub mod config;
pub mod discovery;
pub mod import;
pub mod init;
pub mod manifest;

pub use check::{CheckError, CheckReport, check_project};
pub use config::ProjectConfig;
pub use discovery::{
    DetectedBuildSystem, IGNORED_DIRECTORY_NAMES, ProjectDiscovery, ProjectDiscoveryError,
    discover_project,
};
pub use import::{
    IMPORT_METADATA_RELATIVE_PATH, ImportError, ImportMetadata, ImportMetadataError, ImportOptions,
    ImportSummary, format_import_report, import_project,
};
pub use init::{
    InitError, InitOptions, InitSummary, derive_package_name, init_project,
    init_project_with_options, sanitize_header_dir_name,
};
pub use manifest::{
    MANIFEST_FILENAME, Manifest, ManifestError, ManifestValidationError, ProjectLanguage,
    default_standard_for_language, is_valid_c_standard, is_valid_cpp_standard, normalize_standard,
};
