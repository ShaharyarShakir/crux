//! Dependency resolution engine, catalog abstraction, and lockfile synchronization.

pub mod catalog;
pub mod lockfile;
pub mod resolver;
pub mod source;

pub use catalog::{
    CatalogEntry, CatalogError, InMemoryCatalog, PackageCatalog, default_dev_catalog,
};
pub use lockfile::{
    CURRENT_LOCKFILE_VERSION, Checksum, ChecksumAlgorithm, ChecksumError, LOCKFILE_FILENAME,
    LockCheckReport, LockError, LockSyncError, LockedPackage, Lockfile, LockfileValidationError,
    PackageSource, check_lockfile, check_lockfile_sync, write_lockfile_atomically,
};
pub use resolver::{
    DependencyEdge, RequirementSource, ResolutionError, ResolvedGraph, ResolvedPackage, Resolver,
    UnmatchedRequirement,
};
pub use source::{
    DirectSource, GitRef, SourceError, normalize_direct_path, relative_source_path,
    validate_archive_url, validate_git_ref, validate_git_url,
};
