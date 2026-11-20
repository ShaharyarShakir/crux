//! Direct-source catalog overlay for Git, archive, and local-path dependencies.
//!
//! Direct sources are normalized into the same `CatalogEntry`, `PackageSource`,
//! and `PackageDefinition` models used by registry packages. The overlay is
//! assembled before resolution so the resolver remains storage-agnostic.

use super::{AcquiredSource, AcquisitionError, SourceAcquirer};
use crate::catalog::{CatalogEntry, CatalogError, PackageCatalog};
use crate::dependency::Dependency;
use crate::lock::{LockedPackage, Lockfile};
use crate::package::{PackageId, PackageName};
use crate::registry::{
    GitRegistry, PACKAGE_DEFINITION_FILENAME, PackageDefinition, PackageSourceDefinition,
    load_config, parse_package_definition,
};
use crate::resolution::{DirectSource, PackageSource, relative_source_path};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// A catalog composed of the active registry plus validated direct sources.
#[derive(Debug, Clone)]
pub struct ActiveCatalog {
    registry: GitRegistry,
    direct: DirectCatalog,
}

impl ActiveCatalog {
    /// Loads the active registry and overlays all direct sources reachable from
    /// `extra_dependencies` (in addition to the manifest's dependencies).
    pub fn load(
        project_dir: &Path,
        registry: GitRegistry,
        extra_dependencies: &[Dependency],
        existing_lockfile: Option<&Lockfile>,
        refresh_direct_sources: bool,
    ) -> Result<Self, AcquisitionError> {
        let config = load_config(Some(project_dir))
            .map_err(|err| AcquisitionError::Registry(Box::new(err)))?;
        let cache =
            crate::source::SourceCache::from_config(&config).map_err(AcquisitionError::Io)?;
        let acquirer = SourceAcquirer::new(cache);

        let manifest_path = project_dir.join(crate::project::manifest::MANIFEST_FILENAME);
        let mut dependencies = if manifest_path.is_file() {
            crate::project::manifest::Manifest::load_from_path(&manifest_path)
                .map_err(|err| AcquisitionError::PackageMetadata(err.to_string()))?
                .dependencies
        } else {
            // `crux lock --check` intentionally supports validating a lockfile
            // in a directory without a manifest.
            Vec::new()
        };
        dependencies.extend(extra_dependencies.iter().cloned());

        let direct = DirectCatalog::load(
            &acquirer,
            &registry,
            project_dir,
            &dependencies,
            existing_lockfile,
            refresh_direct_sources,
        )?;
        Ok(Self { registry, direct })
    }

    pub fn registry(&self) -> &GitRegistry {
        &self.registry
    }

    pub fn registry_name(&self) -> &str {
        self.registry.name()
    }

    /// Returns the package definition for an id, preferring direct sources.
    pub fn definition(&self, id: &PackageId) -> Option<&PackageDefinition> {
        self.direct
            .definitions
            .get(id)
            .or_else(|| self.registry.get_package_definition(id))
    }

    /// Returns the canonical source identity recorded for a resolved package.
    pub fn source_for(&self, id: &PackageId) -> Option<&PackageSource> {
        self.direct.sources.get(id)
    }

    /// Returns all package names known to the active catalog.
    pub fn package_names(&self) -> Vec<PackageName> {
        let mut names: BTreeSet<PackageName> = self.direct.entries.keys().cloned().collect();
        names.extend(self.registry.package_names());
        names.into_iter().collect()
    }

    /// Resolves a package name case-insensitively when possible.
    pub fn resolve_name(&self, name: &PackageName) -> PackageName {
        if self.direct.entries.contains_key(name) || self.registry.package_names().contains(name) {
            return name.clone();
        }
        self.package_names()
            .into_iter()
            .find(|candidate| candidate.as_str().eq_ignore_ascii_case(name.as_str()))
            .unwrap_or_else(|| name.clone())
    }

    /// Acquires a package definition using the active catalog's source identity.
    pub fn acquire_definition(
        &self,
        acquirer: &SourceAcquirer,
        project_dir: &Path,
        id: &PackageId,
        force: bool,
    ) -> Result<AcquiredSource, AcquisitionError> {
        let definition = self.definition(id).ok_or_else(|| {
            AcquisitionError::PackageMetadata(format!(
                "no package definition available for package '{id}'"
            ))
        })?;
        let source_definition = if let Some(source) = self.source_for(id) {
            PackageSourceDefinition::from_package_source(source)
                .map_err(AcquisitionError::InvalidSource)?
        } else {
            definition.source.clone()
        };
        acquirer.acquire_direct(id, &source_definition, project_dir, force)
    }

    /// Acquires a locked package using its recorded source identity.
    pub fn acquire_locked(
        &self,
        acquirer: &SourceAcquirer,
        project_dir: &Path,
        locked: &LockedPackage,
        force: bool,
    ) -> Result<AcquiredSource, AcquisitionError> {
        self.acquire_definition(acquirer, project_dir, &locked.id(), force)
    }
}

impl PackageCatalog for ActiveCatalog {
    fn get_package_versions(
        &self,
        package: &PackageName,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        let direct = self.direct.get_entries(package);
        if !direct.is_empty() {
            return Ok(direct);
        }
        self.registry.get_package_versions(package)
    }
}

/// Validated direct-source catalog entries and their package definitions.
#[derive(Debug, Clone, Default)]
pub struct DirectCatalog {
    entries: BTreeMap<PackageName, Vec<CatalogEntry>>,
    definitions: BTreeMap<PackageId, PackageDefinition>,
    sources: BTreeMap<PackageId, PackageSource>,
    name_sources: BTreeMap<PackageName, PackageSource>,
}

impl DirectCatalog {
    /// Builds a direct-source overlay from all direct dependencies in `dependencies`.
    pub fn load(
        acquirer: &SourceAcquirer,
        registry: &GitRegistry,
        project_dir: &Path,
        dependencies: &[Dependency],
        existing_lockfile: Option<&Lockfile>,
        refresh_direct_sources: bool,
    ) -> Result<Self, AcquisitionError> {
        let mut catalog = Self::default();
        let mut state = LoadState {
            acquirer,
            registry,
            project_dir,
            existing_lockfile,
            refresh_direct_sources,
            visiting: BTreeSet::new(),
            visited_paths: BTreeSet::new(),
        };

        for dep in dependencies {
            state.load_dependency(dep, project_dir, &mut catalog)?;
        }
        Ok(catalog)
    }

    fn get_entries(&self, package: &PackageName) -> Vec<CatalogEntry> {
        self.entries.get(package).cloned().unwrap_or_default()
    }
}

struct LoadState<'a> {
    acquirer: &'a SourceAcquirer,
    registry: &'a GitRegistry,
    project_dir: &'a Path,
    existing_lockfile: Option<&'a Lockfile>,
    refresh_direct_sources: bool,
    visiting: BTreeSet<String>,
    visited_paths: BTreeSet<PathBuf>,
}

impl LoadState<'_> {
    fn load_dependency(
        &mut self,
        dep: &Dependency,
        base_dir: &Path,
        catalog: &mut DirectCatalog,
    ) -> Result<(), AcquisitionError> {
        let Some(declared_source) = dep.source() else {
            return Ok(());
        };
        let declared = declared_source
            .direct()
            .ok_or_else(|| AcquisitionError::InvalidSource(declared_source.to_string()))?;

        if !self
            .registry
            .get_package_versions(dep.name())
            .map_err(|err| AcquisitionError::InvalidSource(err.to_string()))?
            .is_empty()
        {
            return Err(AcquisitionError::SourceConflict(format!(
                "package '{}' is declared as a direct source but is also present in registry '{}'",
                dep.name(),
                self.registry.name()
            )));
        }

        let effective_source = self.effective_source(dep.name(), &declared);
        let effective_direct = effective_source.direct().ok_or_else(|| {
            AcquisitionError::InvalidSource(format!("invalid direct source '{effective_source}'"))
        })?;

        let cycle_key = match &effective_direct {
            DirectSource::Path { path } => {
                let canonical = self.resolve_path(path, base_dir)?;
                if !self.visited_paths.insert(canonical.clone()) {
                    return Err(AcquisitionError::SourceCycle(format!(
                        "local path '{}' appears more than once in the direct-source graph",
                        canonical.display()
                    )));
                }
                format!("path:{}", canonical.display())
            }
            other => format!("direct:{}", other.identity_key()),
        };

        if !self.visiting.insert(cycle_key.clone()) {
            return Err(AcquisitionError::SourceCycle(format!(
                "source '{}' for package '{}' is part of a dependency cycle",
                effective_source,
                dep.name()
            )));
        }
        let result = self.load_dependency_inner(dep, &effective_source, catalog);
        self.visiting.remove(&cycle_key);
        if let DirectSource::Path { path } = &effective_direct {
            let canonical = self.resolve_path(path, base_dir)?;
            self.visited_paths.remove(&canonical);
        }
        result
    }

    fn load_dependency_inner(
        &mut self,
        dep: &Dependency,
        effective_source: &PackageSource,
        catalog: &mut DirectCatalog,
    ) -> Result<(), AcquisitionError> {
        let source_definition = PackageSourceDefinition::from_package_source(effective_source)
            .map_err(AcquisitionError::InvalidSource)?;
        let provisional_id = PackageId::new(
            dep.name().clone(),
            semver::Version::parse("0.0.0").expect("valid placeholder version"),
        );
        let acquired = self.acquirer.acquire_direct(
            &provisional_id,
            &source_definition,
            self.project_dir,
            false,
        )?;

        // Git acquisition resolves mutable refs to an immutable commit. The
        // resolved commit becomes part of the catalog source identity so the
        // resolver and lockfile record exactly what was acquired.
        let locked_source = if let Some(commit) = &acquired.resolved_git_commit {
            let declared = effective_source.direct().ok_or_else(|| {
                AcquisitionError::InvalidSource(format!(
                    "invalid direct source '{effective_source}'"
                ))
            })?;
            let direct = declared.with_resolved_commit(commit).map_err(|err| {
                AcquisitionError::InvalidSource(format!(
                    "failed to attach resolved Git commit to '{effective_source}': {err}"
                ))
            })?;
            PackageSource::new(direct.to_string_canonical())
                .map_err(|err| AcquisitionError::InvalidSource(err.to_string()))?
        } else {
            effective_source.clone()
        };

        let package_toml = acquired.source_dir.join(PACKAGE_DEFINITION_FILENAME);
        if !package_toml.is_file() {
            return Err(AcquisitionError::PackageMetadata(format!(
                "direct source '{}' is missing '{}'",
                effective_source,
                package_toml.display()
            )));
        }
        let text = fs::read_to_string(&package_toml)?;
        let mut definition =
            parse_package_definition(&text, &package_toml, Some(source_definition))
                .map_err(|err| AcquisitionError::PackageMetadata(err.to_string()))?;

        if definition.metadata.name != *dep.name() {
            return Err(AcquisitionError::SourceConflict(format!(
                "dependency '{}' resolves to package '{}' from source '{}'",
                dep.name(),
                definition.metadata.name,
                effective_source
            )));
        }
        if !dep.version_req().matches(&definition.metadata.version) {
            return Err(AcquisitionError::SourceConflict(format!(
                "package '{}' version {} from source '{}' does not satisfy requirement '{}'",
                dep.name(),
                definition.metadata.version,
                effective_source,
                dep.version_req()
            )));
        }

        if let Some(existing_source) = catalog.name_sources.get(&definition.metadata.name) {
            if existing_source.identity_key() != effective_source.identity_key() {
                return Err(AcquisitionError::SourceConflict(format!(
                    "package '{}' is provided by conflicting direct sources '{existing_source}' and '{effective_source}'",
                    definition.metadata.name
                )));
            }
        }

        let id = definition.metadata.id();
        let entry = CatalogEntry::new(
            definition.metadata.name.clone(),
            definition.metadata.version.clone(),
        )
        .with_source(locked_source.clone())
        .with_checksum(acquired.checksum.clone())
        .with_dependencies(definition.dependencies.clone());

        if let Some(existing) = catalog.definitions.get(&id) {
            if existing.source != definition.source {
                return Err(AcquisitionError::SourceConflict(format!(
                    "package '{id}' is provided by conflicting sources '{:?}' and '{:?}'",
                    existing.source, definition.source
                )));
            }
            return Ok(());
        }

        catalog
            .entries
            .entry(definition.metadata.name.clone())
            .or_default()
            .push(entry);
        catalog.definitions.insert(id.clone(), definition.clone());
        catalog
            .name_sources
            .entry(definition.metadata.name.clone())
            .or_insert_with(|| locked_source.clone());
        catalog.sources.insert(id, locked_source);

        for nested in &mut definition.dependencies {
            let rebased = rebase_path_dependency(nested, &acquired.source_dir, self.project_dir)?;
            self.load_dependency(&rebased, &acquired.source_dir, catalog)?;
        }
        Ok(())
    }

    fn effective_source(&self, name: &PackageName, declared: &DirectSource) -> PackageSource {
        if self.refresh_direct_sources {
            return PackageSource::new(declared.to_string_canonical())
                .expect("canonical direct source is valid");
        }

        self.existing_lockfile
            .and_then(|lockfile| lockfile.packages.iter().find(|pkg| pkg.name == *name))
            .and_then(|locked| locked.source.direct())
            .filter(|locked| locked.identity_key() == declared.identity_key())
            .map(|locked| {
                PackageSource::new(locked.to_string_canonical()).expect("valid locked source")
            })
            .unwrap_or_else(|| {
                PackageSource::new(declared.to_string_canonical())
                    .expect("canonical direct source is valid")
            })
    }

    fn resolve_path(&self, path: &str, base_dir: &Path) -> Result<PathBuf, AcquisitionError> {
        let normalized = crate::resolution::normalize_direct_path(path);
        let candidate = if normalized.is_absolute() {
            normalized
        } else {
            base_dir.join(normalized)
        };
        fs::canonicalize(&candidate).map_err(|err| {
            AcquisitionError::InvalidSource(format!(
                "local package path '{}' cannot be resolved: {err}",
                candidate.display()
            ))
        })
    }
}

fn rebase_path_dependency(
    dep: &Dependency,
    source_dir: &Path,
    project_dir: &Path,
) -> Result<Dependency, AcquisitionError> {
    let Some(source) = dep.source() else {
        return Ok(dep.clone());
    };
    let Some(DirectSource::Path { path }) = source.direct() else {
        return Ok(dep.clone());
    };
    if path.starts_with('/') {
        return Ok(dep.clone());
    }
    let resolved = fs::canonicalize(source_dir.join(&path)).map_err(|err| {
        AcquisitionError::InvalidSource(format!(
            "local dependency path '{path}' from '{}' cannot be resolved: {err}",
            source_dir.display()
        ))
    })?;
    let relative = relative_source_path(project_dir, &resolved).ok_or_else(|| {
        AcquisitionError::InvalidSource(format!(
            "local dependency path '{}' cannot be expressed relative to the project directory",
            resolved.display()
        ))
    })?;
    Ok(
        Dependency::new(dep.name().clone(), dep.version_req().clone()).with_source(
            PackageSource::try_path(&relative)
                .map_err(|err| AcquisitionError::InvalidSource(err.to_string()))?,
        ),
    )
}
