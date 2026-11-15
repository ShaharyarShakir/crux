//! Dependency resolution engine, catalog abstraction, and lockfile synchronization.

pub mod catalog;
pub mod lockfile;
pub mod resolver;

pub use catalog::{
    CatalogEntry, CatalogError, InMemoryCatalog, PackageCatalog, default_dev_catalog,
};
pub use lockfile::{
    CURRENT_LOCKFILE_VERSION, Checksum, ChecksumAlgorithm, ChecksumError, LOCKFILE_FILENAME,
    LockCheckReport, LockError, LockSyncError, LockedPackage, Lockfile, LockfileValidationError,
    PackageSource, SourceError, check_lockfile, check_lockfile_sync, write_lockfile_atomically,
};
pub use resolver::{
    DependencyEdge, RequirementSource, ResolutionError, ResolvedGraph, ResolvedPackage, Resolver,
    UnmatchedRequirement,
};
