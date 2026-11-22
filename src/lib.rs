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
pub mod ui;

pub use build::{
    AdhocRunPlan, AppBuildEngine, AppBuildLayout, AppBuildModel, AppBuildRequest, AppBuildResult,
    AppCMakeConfig, AppDependency, AppDependencyPlan, AppTarget, AppTargetKind, ArtifactType,
    BuildArtifact, BuildConfiguration, BuildEngine, BuildError, BuildFingerprint, BuildMetadata,
    BuildProfile, BuildRequest, BuildResult, CMakeBuildEngine, CMakeInfo, CompilerFamily,
    CompilerInfo, DiscoveredSources, DiscoveryError, PlatformInfo, ProjectToolchain,
    ToolchainError, ToolchainInfo, detect_compiler, detect_compiler_with_custom,
    detect_project_toolchain, detect_project_toolchain_with_custom, discover_sources,
    discover_sources_with_roots, generate_app_cmakelists, minimum_cmake_version,
    plan_app_dependencies, prepare_adhoc_build, probe_standard_support, sanitize_target_name,
    standard_to_cmake_standard,
};

pub use catalog::{
    CatalogEntry, CatalogError, InMemoryCatalog, PackageCatalog, default_dev_catalog,
};
pub use dependency::{Dependency, DependencyError};
pub use init::{InitError, InitOptions, InitSummary, init_project, init_project_with_options};
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
pub use manifest::{Manifest, ManifestError, ManifestValidationError, ProjectLanguage};
pub use package::{PackageId, PackageMetadata, PackageName};
pub use project::{
    DetectedBuildSystem, IMPORT_METADATA_RELATIVE_PATH, ImportError, ImportMetadata,
    ImportMetadataError, ImportOptions, ImportSummary, ProjectConfig, ProjectDiscovery,
    ProjectDiscoveryError, format_import_report, import_project,
};
pub use registry::{
    CruxConfig, GitOps, GitRegistry, OFFICIAL_REGISTRY_DIR_NAME, OFFICIAL_REGISTRY_NAME,
    OFFICIAL_REGISTRY_SUBDIR, OFFICIAL_REGISTRY_URL, PackageDefinition, PackageSourceDefinition,
    RegistryConfig, RegistryError, RegistryManifest, RegistryStatus, add_registry,
    ensure_official_registry, init_dev_fixture_test_git_registry, init_test_git_registry,
    list_registries, load_config, official_registry_cache_dir, official_registry_url,
    registries_dir_path, save_config, seed_dev_fixture_registry_files, seed_registry_files,
    update_registry,
};
pub use resolution::{
    DirectSource, GitRef, normalize_direct_path, relative_source_path, validate_archive_url,
    validate_git_ref, validate_git_url,
};
pub use resolver::{
    DependencyEdge, RequirementSource, ResolutionError, ResolvedGraph, ResolvedPackage, Resolver,
    UnmatchedRequirement,
};
pub use source::{
    AcquiredSource, AcquisitionError, ArchiveFormat, ExtractionLimits, HttpTransport,
    MockHttpTransport, MockResponse, SourceAcquirer, SourceCache, UreqHttpTransport,
};
