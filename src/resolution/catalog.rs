//! Package catalog abstraction and in-memory development test fixture catalog.

use crate::dependency::Dependency;
use crate::lock::{Checksum, PackageSource};
use crate::package::{PackageId, PackageName};
use std::collections::BTreeMap;
use std::fmt;

/// An entry in a package catalog representing an available package version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub name: PackageName,
    pub version: semver::Version,
    pub source: PackageSource,
    pub checksum: Option<Checksum>,
    pub dependencies: Vec<Dependency>,
}

impl CatalogEntry {
    /// Creates a new catalog entry with default registry source and no dependencies.
    pub fn new(name: PackageName, version: semver::Version) -> Self {
        Self {
            name,
            version,
            source: PackageSource::default_registry(),
            checksum: None,
            dependencies: Vec::new(),
        }
    }

    /// Returns the unique `PackageId` for this entry.
    pub fn id(&self) -> PackageId {
        PackageId::new(self.name.clone(), self.version.clone())
    }

    /// Sets the package source.
    pub fn with_source(mut self, source: PackageSource) -> Self {
        self.source = source;
        self
    }

    /// Sets the integrity checksum.
    pub fn with_checksum(mut self, checksum: Checksum) -> Self {
        self.checksum = Some(checksum);
        self
    }

    /// Sets the declared dependencies.
    pub fn with_dependencies(mut self, mut dependencies: Vec<Dependency>) -> Self {
        dependencies.sort();
        self.dependencies = dependencies;
        self
    }

    /// Adds a declared dependency.
    pub fn add_dependency(&mut self, dep: Dependency) {
        self.dependencies.push(dep);
        self.dependencies.sort();
    }
}

/// Errors related to catalog definition, querying, or population.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    DuplicateEntry(PackageId),
    InvalidEntry(String),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CatalogError::DuplicateEntry(id) => {
                write!(f, "duplicate catalog entry for package '{id}'")
            }
            CatalogError::InvalidEntry(msg) => write!(f, "invalid catalog entry: {msg}"),
        }
    }
}

impl std::error::Error for CatalogError {}

/// Trait defining access to package metadata and version listings.
pub trait PackageCatalog {
    /// Returns all available versions for a given package name.
    fn get_package_versions(
        &self,
        package: &PackageName,
    ) -> Result<Vec<CatalogEntry>, CatalogError>;
}

/// In-memory catalog implementation for testing and development.
#[derive(Debug, Clone, Default)]
pub struct InMemoryCatalog {
    packages: BTreeMap<PackageName, BTreeMap<semver::Version, CatalogEntry>>,
}

impl InMemoryCatalog {
    /// Creates an empty in-memory catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a catalog entry. Returns an error if an identical (name, version) already exists.
    pub fn add_entry(&mut self, entry: CatalogEntry) -> Result<(), CatalogError> {
        let pkg_versions = self.packages.entry(entry.name.clone()).or_default();
        if pkg_versions.contains_key(&entry.version) {
            return Err(CatalogError::DuplicateEntry(entry.id()));
        }
        pkg_versions.insert(entry.version.clone(), entry);
        Ok(())
    }

    /// Convenience helper for adding packages with dependencies.
    pub fn add_package(
        &mut self,
        name: &str,
        version: &str,
        deps: &[(&str, &str)],
    ) -> Result<CatalogEntry, CatalogError> {
        let pkg_name = PackageName::new(name).map_err(|e| {
            CatalogError::InvalidEntry(format!("invalid package name '{name}': {e}"))
        })?;
        let semver_ver = semver::Version::parse(version)
            .map_err(|e| CatalogError::InvalidEntry(format!("invalid version '{version}': {e}")))?;

        let mut dependencies = Vec::new();
        for (dep_name, dep_req) in deps {
            let dep = Dependency::try_new(*dep_name, *dep_req).map_err(|e| {
                CatalogError::InvalidEntry(format!("invalid dependency '{dep_name}': {e}"))
            })?;
            dependencies.push(dep);
        }

        let entry = CatalogEntry::new(pkg_name, semver_ver).with_dependencies(dependencies);
        self.add_entry(entry.clone())?;
        Ok(entry)
    }

    /// Returns the total number of distinct packages in the catalog.
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// Returns the total number of distinct package versions across all packages.
    pub fn total_version_count(&self) -> usize {
        self.packages.values().map(|v| v.len()).sum()
    }
}

impl PackageCatalog for InMemoryCatalog {
    fn get_package_versions(
        &self,
        package: &PackageName,
    ) -> Result<Vec<CatalogEntry>, CatalogError> {
        match self.packages.get(package) {
            Some(versions) => Ok(versions.values().cloned().collect()),
            None => Ok(Vec::new()),
        }
    }
}

/// Creates a standard in-memory test fixture catalog for CLI testing and development.
pub fn default_dev_catalog() -> InMemoryCatalog {
    let mut catalog = InMemoryCatalog::new();

    // fmt versions
    catalog.add_package("fmt", "10.0.0", &[]).unwrap();
    catalog.add_package("fmt", "11.0.0", &[]).unwrap();
    catalog.add_package("fmt", "11.1.0", &[]).unwrap();
    catalog.add_package("fmt", "11.1.1", &[]).unwrap();

    // zlib versions
    catalog.add_package("zlib", "1.2.11", &[]).unwrap();
    catalog.add_package("zlib", "1.2.13", &[]).unwrap();
    catalog.add_package("zlib", "1.3.0", &[]).unwrap();
    catalog.add_package("zlib", "1.3.1", &[]).unwrap();

    // openssl versions (depends on zlib)
    catalog
        .add_package("openssl", "3.0.0", &[("zlib", "^1.2")])
        .unwrap();
    catalog
        .add_package("openssl", "3.3.0", &[("zlib", "^1.2")])
        .unwrap();
    catalog
        .add_package("openssl", "3.4.0", &[("zlib", "^1.3")])
        .unwrap();

    // spdlog versions (depends on fmt)
    catalog
        .add_package("spdlog", "1.12.0", &[("fmt", "^10.0")])
        .unwrap();
    catalog
        .add_package("spdlog", "1.13.0", &[("fmt", "^11.1")])
        .unwrap();

    // catch2 versions
    catalog.add_package("catch2", "3.4.0", &[]).unwrap();
    catalog.add_package("catch2", "3.5.0", &[]).unwrap();

    catalog
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_memory_catalog_add_and_query() {
        let mut catalog = InMemoryCatalog::new();
        let entry = catalog.add_package("fmt", "11.1.1", &[]).unwrap();

        assert_eq!(entry.name.as_str(), "fmt");
        assert_eq!(entry.version.to_string(), "11.1.1");
        assert_eq!(catalog.package_count(), 1);
        assert_eq!(catalog.total_version_count(), 1);

        let queried = catalog
            .get_package_versions(&PackageName::new("fmt").unwrap())
            .unwrap();
        assert_eq!(queried.len(), 1);
        assert_eq!(queried[0].version.to_string(), "11.1.1");
    }

    #[test]
    fn test_in_memory_catalog_rejects_duplicates() {
        let mut catalog = InMemoryCatalog::new();
        catalog.add_package("zlib", "1.3.1", &[]).unwrap();
        let err = catalog.add_package("zlib", "1.3.1", &[]);
        assert!(matches!(err, Err(CatalogError::DuplicateEntry(_))));
    }

    #[test]
    fn test_in_memory_catalog_nonexistent_package() {
        let catalog = InMemoryCatalog::new();
        let queried = catalog
            .get_package_versions(&PackageName::new("nonexistent").unwrap())
            .unwrap();
        assert!(queried.is_empty());
    }
}
