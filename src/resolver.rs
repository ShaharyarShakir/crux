//! Deterministic dependency resolution engine for Crux.

use crate::catalog::{CatalogEntry, PackageCatalog};
use crate::dependency::Dependency;
use crate::lock::{Checksum, LockedPackage, Lockfile, LockfileValidationError, PackageSource};
use crate::package::{PackageId, PackageName};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// A resolved package in the dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPackage {
    pub name: PackageName,
    pub version: semver::Version,
    pub source: PackageSource,
    pub checksum: Option<Checksum>,
    pub dependencies: Vec<PackageId>,
}

impl ResolvedPackage {
    /// Returns the canonical `PackageId` for this resolved package.
    pub fn id(&self) -> PackageId {
        PackageId::new(self.name.clone(), self.version.clone())
    }
}

/// A directed edge in the resolved dependency graph (from dependent to target).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyEdge {
    pub from: PackageName,
    pub to: PackageName,
    pub requirement: semver::VersionReq,
}

impl PartialOrd for DependencyEdge {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DependencyEdge {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.from
            .cmp(&other.from)
            .then_with(|| self.to.cmp(&other.to))
            .then_with(|| {
                self.requirement
                    .to_string()
                    .cmp(&other.requirement.to_string())
            })
    }
}

/// The complete resolved dependency graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGraph {
    pub root_package: Option<PackageId>,
    pub root_dependencies: Vec<Dependency>,
    pub packages: BTreeMap<PackageName, ResolvedPackage>,
    pub edges: Vec<DependencyEdge>,
}

impl ResolvedGraph {
    /// Looks up a resolved package by name.
    pub fn get(&self, name: &str) -> Option<&ResolvedPackage> {
        self.packages.get(name)
    }

    /// Returns the total count of resolved packages.
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }

    /// Converts the resolved graph into a valid Crux `Lockfile`.
    pub fn to_lockfile(&self) -> Result<Lockfile, LockfileValidationError> {
        self.to_lockfile_with_existing(None)
    }

    /// Converts the resolved graph into a valid Crux `Lockfile`, preserving
    /// existing checksums and sources from a previous lockfile if matching.
    pub fn to_lockfile_with_existing(
        &self,
        existing: Option<&Lockfile>,
    ) -> Result<Lockfile, LockfileValidationError> {
        let mut lockfile = Lockfile::new();
        for pkg in self.packages.values() {
            let mut source = pkg.source.clone();
            let mut checksum = pkg.checksum.clone();

            if let Some(prev_pkg) = existing.and_then(|prev| {
                prev.packages
                    .iter()
                    .find(|p| p.name == pkg.name && p.version == pkg.version)
            }) {
                if checksum.is_none() && prev_pkg.checksum.is_some() {
                    checksum = prev_pkg.checksum.clone();
                }
                if source == PackageSource::default_registry()
                    && prev_pkg.source != PackageSource::default_registry()
                {
                    source = prev_pkg.source.clone();
                }
            }

            let mut locked = LockedPackage::new(pkg.name.clone(), pkg.version.clone(), source)
                .with_dependencies(pkg.dependencies.clone());

            if let Some(cs) = checksum {
                locked = locked.with_checksum(cs);
            }

            lockfile.add_package(locked)?;
        }
        Ok(lockfile)
    }
}

/// Identifies the source of a version constraint (root manifest or a parent dependency).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementSource {
    pub dependent: Option<PackageId>,
    pub requirement: semver::VersionReq,
}

/// Details regarding a version requirement that could not be matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnmatchedRequirement {
    pub package: PackageName,
    pub requirement: semver::VersionReq,
    pub available: Vec<semver::Version>,
    pub dependent: Option<PackageId>,
}

/// Errors that can occur during dependency resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionError {
    /// Package not found in catalog.
    PackageNotFound {
        package: PackageName,
        dependent: Option<PackageId>,
    },
    /// Package exists in catalog, but has no versions published.
    NoAvailableVersions {
        package: PackageName,
        dependent: Option<PackageId>,
    },
    /// No version in the catalog satisfies the version requirement.
    NoMatchingVersion(Box<UnmatchedRequirement>),
    /// Two or more dependencies require incompatible versions of the same package.
    ConflictingRequirements {
        package: PackageName,
        conflicts: Vec<RequirementSource>,
    },
    /// A circular dependency was detected during graph traversal.
    CyclicDependency { cycle: Vec<PackageName> },
    /// Catalog returned an error (e.g. duplicate or malformed entry).
    Catalog(String),
}

impl ResolutionError {
    /// Helper to construct a `NoMatchingVersion` error without manual boxing.
    pub fn no_matching_version(
        package: PackageName,
        requirement: semver::VersionReq,
        available: Vec<semver::Version>,
        dependent: Option<PackageId>,
    ) -> Self {
        ResolutionError::NoMatchingVersion(Box::new(UnmatchedRequirement {
            package,
            requirement,
            available,
            dependent,
        }))
    }
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResolutionError::PackageNotFound { package, dependent } => match dependent {
                Some(dep) => {
                    write!(
                        f,
                        "package '{package}' not found in catalog (requested by {dep})"
                    )
                }
                None => write!(f, "package '{package}' not found in catalog"),
            },
            ResolutionError::NoAvailableVersions { package, dependent } => match dependent {
                Some(dep) => {
                    write!(
                        f,
                        "no versions available for package '{package}' (requested by {dep})"
                    )
                }
                None => write!(f, "no versions available for package '{package}'"),
            },
            ResolutionError::NoMatchingVersion(details) => {
                let avail_str = if details.available.is_empty() {
                    "none".to_string()
                } else {
                    details
                        .available
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                match &details.dependent {
                    Some(dep) => write!(
                        f,
                        "no version of package '{}' matches requirement '{}' (available: [{avail_str}], requested by {dep})",
                        details.package, details.requirement
                    ),
                    None => write!(
                        f,
                        "no version of package '{}' matches requirement '{}' (available: [{avail_str}])",
                        details.package, details.requirement
                    ),
                }
            }
            ResolutionError::ConflictingRequirements { package, conflicts } => {
                write!(
                    f,
                    "conflicting version requirements for package '{package}':"
                )?;
                for req in conflicts {
                    match &req.dependent {
                        Some(dep) => write!(f, "\n  - {dep} requires {}", req.requirement)?,
                        None => write!(f, "\n  - root manifest requires {}", req.requirement)?,
                    }
                }
                Ok(())
            }
            ResolutionError::CyclicDependency { cycle } => {
                let cycle_str = cycle
                    .iter()
                    .map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(" -> ");
                write!(f, "cyclic dependency detected: {cycle_str}")
            }
            ResolutionError::Catalog(err) => write!(f, "catalog error: {err}"),
        }
    }
}

impl std::error::Error for ResolutionError {}

/// Internal resolution context tracking assigned packages, accumulated constraints, and edges.
#[derive(Clone)]
struct ResolutionContext {
    selected: BTreeMap<PackageName, CatalogEntry>,
    constraints: BTreeMap<PackageName, Vec<RequirementSource>>,
    edges: Vec<DependencyEdge>,
}

/// Helper function to detect directed dependency cycles using BFS.
fn find_cycle_path(
    from: &PackageName,
    to: &PackageName,
    edges: &[DependencyEdge],
) -> Option<Vec<PackageName>> {
    if from == to {
        return Some(vec![from.clone(), to.clone()]);
    }

    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    let mut parent: BTreeMap<PackageName, PackageName> = BTreeMap::new();

    queue.push_back(to.clone());
    visited.insert(to.clone());

    while let Some(current) = queue.pop_front() {
        if &current == from {
            // Reconstruct path from `to` to `from`
            let mut path = Vec::new();
            let mut curr = current;
            while let Some(prev) = parent.get(&curr) {
                path.push(curr.clone());
                curr = prev.clone();
            }
            path.push(to.clone());
            path.reverse();

            // Full cycle: from -> to -> ... -> from
            let mut full_cycle = vec![from.clone()];
            full_cycle.extend(path);
            return Some(full_cycle);
        }

        for edge in edges {
            if edge.from == current && !visited.contains(&edge.to) {
                visited.insert(edge.to.clone());
                parent.insert(edge.to.clone(), current.clone());
                queue.push_back(edge.to.clone());
            }
        }
    }

    None
}

/// Deterministic dependency resolver.
pub struct Resolver<'a> {
    catalog: &'a dyn PackageCatalog,
}

impl<'a> Resolver<'a> {
    /// Creates a new resolver backed by the specified package catalog.
    pub fn new(catalog: &'a dyn PackageCatalog) -> Self {
        Self { catalog }
    }

    /// Resolves the declared root dependencies into a complete, deterministic dependency graph.
    pub fn resolve(
        &self,
        root_package: Option<PackageId>,
        root_dependencies: &[Dependency],
    ) -> Result<ResolvedGraph, ResolutionError> {
        self.resolve_with_preferences(root_package, root_dependencies, &BTreeMap::new())
    }

    /// Resolves the declared root dependencies into a complete, deterministic dependency graph,
    /// giving preference to previously locked package versions when they satisfy all active constraints.
    pub fn resolve_with_preferences(
        &self,
        root_package: Option<PackageId>,
        root_dependencies: &[Dependency],
        preferences: &BTreeMap<PackageName, semver::Version>,
    ) -> Result<ResolvedGraph, ResolutionError> {
        let mut context = ResolutionContext {
            selected: BTreeMap::new(),
            constraints: BTreeMap::new(),
            edges: Vec::new(),
        };

        // Register initial root constraints
        for dep in root_dependencies {
            context
                .constraints
                .entry(dep.name().clone())
                .or_default()
                .push(RequirementSource {
                    dependent: root_package.clone(),
                    requirement: dep.version_req().clone(),
                });
        }

        // Distinct, alphabetically sorted pending packages
        let mut pending: BTreeSet<PackageName> =
            root_dependencies.iter().map(|d| d.name().clone()).collect();

        self.solve(&mut pending, &mut context, preferences)?;

        // Construct ResolvedGraph
        let mut packages = BTreeMap::new();
        for (pkg_name, entry) in &context.selected {
            // Find resolved PackageId for each direct dependency of this entry
            let mut resolved_deps = Vec::new();
            for dep in &entry.dependencies {
                if let Some(target) = context.selected.get(dep.name()) {
                    resolved_deps.push(target.id());
                }
            }
            resolved_deps.sort();

            packages.insert(
                pkg_name.clone(),
                ResolvedPackage {
                    name: entry.name.clone(),
                    version: entry.version.clone(),
                    source: entry.source.clone(),
                    checksum: entry.checksum.clone(),
                    dependencies: resolved_deps,
                },
            );
        }

        context.edges.sort();
        context.edges.dedup();

        Ok(ResolvedGraph {
            root_package,
            root_dependencies: root_dependencies.to_vec(),
            packages,
            edges: context.edges,
        })
    }

    /// Recursive backtracking solver.
    fn solve(
        &self,
        pending: &mut BTreeSet<PackageName>,
        context: &mut ResolutionContext,
        preferences: &BTreeMap<PackageName, semver::Version>,
    ) -> Result<(), ResolutionError> {
        if pending.is_empty() {
            return Ok(());
        }

        // Pick next unassigned package deterministically (alphabetically)
        let next_pkg = pending.iter().next().unwrap().clone();
        pending.remove(&next_pkg);

        // Fetch candidate versions from catalog
        let available = self
            .catalog
            .get_package_versions(&next_pkg)
            .map_err(|e| ResolutionError::Catalog(e.to_string()))?;

        let constraints = context
            .constraints
            .get(&next_pkg)
            .cloned()
            .unwrap_or_default();

        let last_dependent = constraints.last().and_then(|c| c.dependent.clone());

        if available.is_empty() {
            return Err(ResolutionError::PackageNotFound {
                package: next_pkg,
                dependent: last_dependent,
            });
        }

        // Filter versions that satisfy ALL constraints on this package
        let mut matching: Vec<CatalogEntry> = available
            .into_iter()
            .filter(|entry| {
                constraints
                    .iter()
                    .all(|c| c.requirement.matches(&entry.version))
            })
            .collect();

        // Sort candidates: if a preferred version exists and matches constraints,
        // it is evaluated first; otherwise, candidates follow SemVer precedence order.
        let pref_opt = preferences.get(&next_pkg);
        matching.sort_by(|a, b| {
            let a_is_pref = pref_opt.is_some_and(|pv| &a.version == pv);
            let b_is_pref = pref_opt.is_some_and(|pv| &b.version == pv);
            match (a_is_pref, b_is_pref) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => b
                    .version
                    .cmp_precedence(&a.version)
                    .then_with(|| b.version.cmp(&a.version)),
            }
        });

        if matching.is_empty() {
            if constraints.len() > 1 {
                return Err(ResolutionError::ConflictingRequirements {
                    package: next_pkg,
                    conflicts: constraints,
                });
            } else {
                let avail_versions = self
                    .catalog
                    .get_package_versions(&next_pkg)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|e| e.version)
                    .collect();
                return Err(ResolutionError::no_matching_version(
                    next_pkg,
                    constraints[0].requirement.clone(),
                    avail_versions,
                    last_dependent,
                ));
            }
        }

        let mut last_err = None;

        // Try candidate versions in descending order
        for candidate in matching {
            let mut snapshot_context = context.clone();
            let mut snapshot_pending = pending.clone();

            snapshot_context
                .selected
                .insert(next_pkg.clone(), candidate.clone());

            let mut cycle_err = None;

            // Process transitive dependencies of candidate
            for dep in &candidate.dependencies {
                // Cycle detection: check if adding edge next_pkg -> dep creates a directed cycle
                if let Some(cycle) = find_cycle_path(&next_pkg, dep.name(), &snapshot_context.edges)
                {
                    cycle_err = Some(ResolutionError::CyclicDependency { cycle });
                    break;
                }

                snapshot_context.edges.push(DependencyEdge {
                    from: next_pkg.clone(),
                    to: dep.name().clone(),
                    requirement: dep.version_req().clone(),
                });

                // Record constraint
                snapshot_context
                    .constraints
                    .entry(dep.name().clone())
                    .or_default()
                    .push(RequirementSource {
                        dependent: Some(candidate.id()),
                        requirement: dep.version_req().clone(),
                    });

                // If already selected, check if selected version violates new constraint
                if let Some(already_selected) = snapshot_context.selected.get(dep.name()) {
                    if !dep.version_req().matches(&already_selected.version) {
                        cycle_err = Some(ResolutionError::ConflictingRequirements {
                            package: dep.name().clone(),
                            conflicts: snapshot_context
                                .constraints
                                .get(dep.name())
                                .cloned()
                                .unwrap_or_default(),
                        });
                        break;
                    }
                } else {
                    snapshot_pending.insert(dep.name().clone());
                }
            }

            if let Some(err) = cycle_err {
                last_err = Some(err);
                continue;
            }

            // Recurse to solve remaining pending packages
            match self.solve(&mut snapshot_pending, &mut snapshot_context, preferences) {
                Ok(()) => {
                    *context = snapshot_context;
                    *pending = snapshot_pending;
                    return Ok(());
                }
                Err(err) => {
                    last_err = Some(err);
                }
            }
        }

        Err(last_err.unwrap_or_else(|| {
            ResolutionError::no_matching_version(
                next_pkg.clone(),
                constraints
                    .first()
                    .map(|c| c.requirement.clone())
                    .unwrap_or(semver::VersionReq::STAR),
                Vec::new(),
                last_dependent,
            )
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::InMemoryCatalog;

    #[test]
    fn test_resolve_exact_version() {
        let mut catalog = InMemoryCatalog::new();
        catalog.add_package("fmt", "11.1.0", &[]).unwrap();
        catalog.add_package("fmt", "11.1.1", &[]).unwrap();

        let resolver = Resolver::new(&catalog);
        let deps = [Dependency::try_new("fmt", "=11.1.0").unwrap()];

        let graph = resolver.resolve(None, &deps).unwrap();
        assert_eq!(graph.package_count(), 1);
        assert_eq!(graph.get("fmt").unwrap().version.to_string(), "11.1.0");
    }

    #[test]
    fn test_resolve_highest_compatible_version() {
        let mut catalog = InMemoryCatalog::new();
        catalog.add_package("zlib", "1.2.11", &[]).unwrap();
        catalog.add_package("zlib", "1.2.13", &[]).unwrap();
        catalog.add_package("zlib", "1.3.0", &[]).unwrap();
        catalog.add_package("zlib", "1.3.1", &[]).unwrap();
        catalog.add_package("zlib", "2.0.0", &[]).unwrap();

        let resolver = Resolver::new(&catalog);
        let deps = [Dependency::try_new("zlib", "^1.2").unwrap()];

        let graph = resolver.resolve(None, &deps).unwrap();
        // ^1.2 matches all 1.x >= 1.2.0; highest is 1.3.1
        assert_eq!(graph.get("zlib").unwrap().version.to_string(), "1.3.1");
    }

    #[test]
    fn test_resolve_tilde_requirement() {
        let mut catalog = InMemoryCatalog::new();
        catalog.add_package("openssl", "3.3.0", &[]).unwrap();
        catalog.add_package("openssl", "3.4.0", &[]).unwrap();
        catalog.add_package("openssl", "3.4.1", &[]).unwrap();
        catalog.add_package("openssl", "3.5.0", &[]).unwrap();

        let resolver = Resolver::new(&catalog);
        let deps = [Dependency::try_new("openssl", "~3.4").unwrap()];

        let graph = resolver.resolve(None, &deps).unwrap();
        // ~3.4 allows >=3.4.0, <3.5.0; highest is 3.4.1
        assert_eq!(graph.get("openssl").unwrap().version.to_string(), "3.4.1");
    }

    #[test]
    fn test_resolve_excludes_prerelease_by_default() {
        let mut catalog = InMemoryCatalog::new();
        catalog.add_package("fmt", "11.0.0", &[]).unwrap();
        catalog.add_package("fmt", "11.1.0-alpha.1", &[]).unwrap();

        let resolver = Resolver::new(&catalog);
        let deps = [Dependency::try_new("fmt", "^11.0").unwrap()];

        let graph = resolver.resolve(None, &deps).unwrap();
        // 11.1.0-alpha.1 is excluded by default
        assert_eq!(graph.get("fmt").unwrap().version.to_string(), "11.0.0");
    }

    #[test]
    fn test_resolve_transitive_linear_chain() {
        let mut catalog = InMemoryCatalog::new();
        catalog
            .add_package("app", "1.0.0", &[("lib_a", "^1.0")])
            .unwrap();
        catalog
            .add_package("lib_a", "1.2.0", &[("lib_b", "^2.0")])
            .unwrap();
        catalog
            .add_package("lib_b", "2.1.0", &[("lib_c", "^3.0")])
            .unwrap();
        catalog.add_package("lib_c", "3.0.0", &[]).unwrap();

        let resolver = Resolver::new(&catalog);
        let root_deps = [Dependency::try_new("lib_a", "^1.0").unwrap()];

        let graph = resolver.resolve(None, &root_deps).unwrap();
        assert_eq!(graph.package_count(), 3);
        assert_eq!(graph.get("lib_a").unwrap().version.to_string(), "1.2.0");
        assert_eq!(graph.get("lib_b").unwrap().version.to_string(), "2.1.0");
        assert_eq!(graph.get("lib_c").unwrap().version.to_string(), "3.0.0");
    }

    #[test]
    fn test_resolve_diamond_dependency_satisfaction() {
        let mut catalog = InMemoryCatalog::new();
        // lib_a requires zlib ^1.2 (permits up to <2.0)
        catalog
            .add_package("lib_a", "1.0.0", &[("zlib", "^1.2")])
            .unwrap();
        // lib_b requires zlib <1.3.0
        catalog
            .add_package("lib_b", "1.0.0", &[("zlib", ">=1.2.8, <1.3.0")])
            .unwrap();
        catalog.add_package("zlib", "1.2.7", &[]).unwrap();
        catalog.add_package("zlib", "1.2.8", &[]).unwrap();
        catalog.add_package("zlib", "1.2.11", &[]).unwrap();
        catalog.add_package("zlib", "1.3.1", &[]).unwrap();

        let resolver = Resolver::new(&catalog);
        let root_deps = [
            Dependency::try_new("lib_a", "^1.0").unwrap(),
            Dependency::try_new("lib_b", "^1.0").unwrap(),
        ];

        let graph = resolver.resolve(None, &root_deps).unwrap();
        assert_eq!(graph.package_count(), 3);
        // zlib must satisfy both ^1.2 and <1.3.0; highest matching is 1.2.11
        assert_eq!(graph.get("zlib").unwrap().version.to_string(), "1.2.11");
    }

    #[test]
    fn test_resolve_conflicting_requirements() {
        let mut catalog = InMemoryCatalog::new();
        catalog
            .add_package("lib_a", "1.0.0", &[("zlib", "^1.3")])
            .unwrap();
        catalog
            .add_package("lib_b", "1.0.0", &[("zlib", "=1.2.11")])
            .unwrap();
        catalog.add_package("zlib", "1.2.11", &[]).unwrap();
        catalog.add_package("zlib", "1.3.1", &[]).unwrap();

        let resolver = Resolver::new(&catalog);
        let root_deps = [
            Dependency::try_new("lib_a", "^1.0").unwrap(),
            Dependency::try_new("lib_b", "^1.0").unwrap(),
        ];

        let err = resolver.resolve(None, &root_deps).unwrap_err();
        assert!(matches!(
            err,
            ResolutionError::ConflictingRequirements { .. }
        ));
    }

    #[test]
    fn test_resolve_missing_transitive_dependency() {
        let mut catalog = InMemoryCatalog::new();
        catalog
            .add_package("lib_a", "1.0.0", &[("ghost_lib", "^1.0")])
            .unwrap();

        let resolver = Resolver::new(&catalog);
        let root_deps = [Dependency::try_new("lib_a", "^1.0").unwrap()];

        let err = resolver.resolve(None, &root_deps).unwrap_err();
        assert!(matches!(err, ResolutionError::PackageNotFound { .. }));
    }

    #[test]
    fn test_resolve_dependency_cycle() {
        let mut catalog = InMemoryCatalog::new();
        catalog
            .add_package("lib_a", "1.0.0", &[("lib_b", "^1.0")])
            .unwrap();
        catalog
            .add_package("lib_b", "1.0.0", &[("lib_a", "^1.0")])
            .unwrap();

        let resolver = Resolver::new(&catalog);
        let root_deps = [Dependency::try_new("lib_a", "^1.0").unwrap()];

        let err = resolver.resolve(None, &root_deps).unwrap_err();
        assert!(matches!(err, ResolutionError::CyclicDependency { .. }));
    }

    #[test]
    fn test_resolved_graph_to_lockfile() {
        let mut catalog = InMemoryCatalog::new();
        catalog.add_package("fmt", "11.1.1", &[]).unwrap();
        catalog.add_package("zlib", "1.3.1", &[]).unwrap();

        let resolver = Resolver::new(&catalog);
        let root_deps = [
            Dependency::try_new("fmt", "^11.1").unwrap(),
            Dependency::try_new("zlib", "^1.3").unwrap(),
        ];

        let graph = resolver.resolve(None, &root_deps).unwrap();
        let lockfile = graph.to_lockfile().unwrap();

        assert_eq!(lockfile.package_count(), 2);
        let toml_str = lockfile.to_toml_string().unwrap();
        assert!(toml_str.contains("lockfile_version = 1"));
        assert!(toml_str.contains("name = \"fmt\""));
        assert!(toml_str.contains("name = \"zlib\""));
    }

    #[test]
    fn test_resolve_with_preferences_preserves_compatible_version() {
        let mut catalog = InMemoryCatalog::new();
        catalog.add_package("fmt", "11.1.0", &[]).unwrap();
        catalog.add_package("fmt", "11.1.1", &[]).unwrap();
        catalog.add_package("fmt", "11.2.0", &[]).unwrap();

        let resolver = Resolver::new(&catalog);
        let root_deps = [Dependency::try_new("fmt", "^11.1").unwrap()];

        // Without preferences, highest compatible is selected: 11.2.0
        let graph_no_pref = resolver.resolve(None, &root_deps).unwrap();
        assert_eq!(
            graph_no_pref.get("fmt").unwrap().version.to_string(),
            "11.2.0"
        );

        // With preference 11.1.0, 11.1.0 is preserved because it matches ^11.1
        let mut prefs = BTreeMap::new();
        prefs.insert(
            PackageName::new("fmt").unwrap(),
            semver::Version::parse("11.1.0").unwrap(),
        );
        let graph_with_pref = resolver
            .resolve_with_preferences(None, &root_deps, &prefs)
            .unwrap();
        assert_eq!(
            graph_with_pref.get("fmt").unwrap().version.to_string(),
            "11.1.0"
        );
    }

    #[test]
    fn test_resolve_with_preferences_upgrades_when_incompatible() {
        let mut catalog = InMemoryCatalog::new();
        catalog.add_package("fmt", "10.0.0", &[]).unwrap();
        catalog.add_package("fmt", "11.1.1", &[]).unwrap();

        let resolver = Resolver::new(&catalog);
        let root_deps = [Dependency::try_new("fmt", "^11.0").unwrap()];

        // Preference is 10.0.0, but manifest requires ^11.0 -> must upgrade to 11.1.1
        let mut prefs = BTreeMap::new();
        prefs.insert(
            PackageName::new("fmt").unwrap(),
            semver::Version::parse("10.0.0").unwrap(),
        );
        let graph = resolver
            .resolve_with_preferences(None, &root_deps, &prefs)
            .unwrap();
        assert_eq!(graph.get("fmt").unwrap().version.to_string(), "11.1.1");
    }

    #[test]
    fn test_resolved_graph_to_lockfile_preserves_checksum_from_existing() {
        let mut catalog = InMemoryCatalog::new();
        catalog.add_package("fmt", "11.1.1", &[]).unwrap();

        let resolver = Resolver::new(&catalog);
        let root_deps = [Dependency::try_new("fmt", "^11.1").unwrap()];
        let graph = resolver.resolve(None, &root_deps).unwrap();

        // Create an existing lockfile containing a checksum for fmt 11.1.1
        let mut existing_lockfile = Lockfile::new();
        let existing_pkg = LockedPackage::new(
            PackageName::new("fmt").unwrap(),
            semver::Version::parse("11.1.1").unwrap(),
            PackageSource::default_registry(),
        )
        .with_checksum(
            Checksum::parse(
                "sha256:d8c6b48bcbe6c507c57dbd8091176b66e33f3893da7e4c29c8e1cb0fcbf1a620",
            )
            .unwrap(),
        );
        existing_lockfile.add_package(existing_pkg).unwrap();

        let lockfile = graph
            .to_lockfile_with_existing(Some(&existing_lockfile))
            .unwrap();
        let locked_fmt = lockfile.packages.first().unwrap();
        assert!(locked_fmt.checksum.is_some());
        assert_eq!(
            locked_fmt.checksum.as_ref().unwrap().to_string(),
            "sha256:d8c6b48bcbe6c507c57dbd8091176b66e33f3893da7e4c29c8e1cb0fcbf1a620"
        );
    }
}
