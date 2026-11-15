//! Package domain model including identity, metadata, and dependencies.

pub mod dependency;
pub mod identity;
pub mod metadata;

pub use dependency::{Dependency, DependencyError};
pub use identity::{
    NameValidationError, PackageId, PackageIdError, PackageName, RESERVED_NAMES,
    validate_package_name,
};
pub use metadata::PackageMetadata;
