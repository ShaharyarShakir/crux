//! Crux: A production-oriented, source-first C/C++ package manager.

pub mod build;
pub mod catalog;
pub mod check;
pub mod cli;
pub mod core;
pub mod dependency;
pub mod init;
pub mod install;
pub mod integrate;
pub mod lock;
pub mod manifest;
pub mod package;
pub mod project;
pub mod registry;
pub mod resolution;
pub mod resolver;
pub mod source;

pub use build::{
    ArtifactType, BuildArtifact, BuildConfiguration, BuildEngine, BuildError, BuildFingerprint,
    BuildMetadata, BuildProfile, BuildRequest, BuildResult, CMakeBuildEngine, ToolchainInfo,
};

pub use catalog::{
    CatalogEntry, CatalogError, InMemoryCatalog, PackageCatalog, default_dev_catalog,
};
pub use dependency::{Dependency, DependencyError};
pub use install::{
    CMakeInstallEngine, CURRENT_INSTALL_METADATA_VERSION, INSTALL_METADATA_FILENAME, InstallEngine,
    InstallError, InstallMetadata, InstallRequest, InstallResult, InstalledFile, InstalledFileType,
};
pub use integrate::{
    CMakeIntegrationGenerator, DEFAULT_CMAKE_INTEGRATION_SUBPATH, IntegrationError,
    IntegrationResult, ValidatedPackageInstallation, collect_transitive_dependencies,
    escape_cmake_path, generate_cmake_dependencies, sanitize_cmake_identifier,
    validate_package_installation, write_cmake_dependencies_atomically,
};
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
