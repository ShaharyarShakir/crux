//! Project management, manifest parsing, initialization, and verification.

pub mod check;
pub mod init;
pub mod manifest;

pub use check::{CheckError, CheckReport, check_project};
pub use init::{InitError, InitSummary, derive_package_name, init_project};
pub use manifest::{MANIFEST_FILENAME, Manifest, ManifestError, ManifestValidationError};
