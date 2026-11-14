//! Crux: A production-oriented, source-first C/C++ package manager.

pub mod catalog;
pub mod check;
pub mod cli;
pub mod dependency;
pub mod init;
pub mod lock;
pub mod manifest;
pub mod package;
pub mod registry;
pub mod resolver;
pub mod source;

pub use catalog::{
    CatalogEntry, CatalogError, InMemoryCatalog, PackageCatalog, default_dev_catalog,
};
pub use dependency::{Dependency, DependencyError};
pub use lock::{
    Checksum, ChecksumAlgorithm, LockCheckReport, LockError, LockSyncError, LockedPackage,
    Lockfile, PackageSource, check_lockfile, check_lockfile_sync, write_lockfile_atomically,
};
pub use package::{PackageId, PackageMetadata, PackageName};
pub use registry::{
    CruxConfig, GitOps, GitRegistry, PackageDefinition, PackageSourceDefinition, RegistryConfig,
    RegistryError, RegistryManifest, RegistryStatus, add_registry, list_registries, load_config,
    update_registry,
};
pub use resolver::{
    DependencyEdge, RequirementSource, ResolutionError, ResolvedGraph, ResolvedPackage, Resolver,
    UnmatchedRequirement,
};
pub use source::{
    AcquiredSource, AcquisitionError, ArchiveFormat, ExtractionLimits, HttpTransport,
    MockHttpTransport, MockResponse, SourceAcquirer, SourceCache, UreqHttpTransport,
};
