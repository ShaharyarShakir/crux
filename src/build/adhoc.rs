//! Planning support for ad-hoc `crux run --bin` / `--sources` invocations.
//!
//! Ad-hoc runs deliberately do not use a `crux.toml` manifest or a package
//! registry. Instead, the CLI-provided source and header files are copied into
//! an isolated Crux-managed scratch workspace under `.crux/run/`, and a normal
//! [`AppBuildModel`](super::AppBuildModel) is synthesized. The same CMake
//! generator and build engine used by project applications therefore handle
//! ad-hoc runs, including Debug/Release profiles and dependency-free linking.

use super::BuildError;
use super::app::{AppBuildLayout, AppBuildModel, AppTarget, CMakeInfo, detect_cmake};
use super::dependency::AppDependencyPlan;
use super::discovery::DiscoveredSources;
use super::generator::sanitize_target_name;
use super::metadata::BuildProfile;
use super::toolchain::{ToolchainError, detect_project_toolchain};
use crate::project::config::ProjectConfig;
use crate::project::manifest::{
    ProjectLanguage, default_standard_for_language, normalize_standard,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

const HEADER_EXTENSIONS: &[&str] = &["h", "hpp", "hxx", "hh", "inl", "ipp"];

/// Fully resolved inputs for one ad-hoc run invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdhocRunPlan {
    /// Crux-managed scratch workspace that owns all copied inputs and artifacts.
    pub workspace_root: PathBuf,
    /// Effective application language inferred from compilation units.
    pub language: ProjectLanguage,
    /// Sanitized executable/target name.
    pub target_name: String,
}

/// Prepares an isolated ad-hoc build model from explicit source inputs.
///
/// The scratch workspace is cleared before materials are copied so a failed
/// build can never leave an executable from an earlier invocation behind.
pub fn prepare_adhoc_build(
    workspace_dir: &Path,
    sources: &[String],
    include_dirs: &[String],
    profile: BuildProfile,
) -> Result<(AppBuildModel, AdhocRunPlan), BuildError> {
    if sources.is_empty() {
        return Err(BuildError::InvalidConfiguration(
            "ad-hoc run requires at least one source file; use --bin or --sources".to_string(),
        ));
    }

    let workspace_root = workspace_dir
        .canonicalize()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => BuildError::InvalidConfiguration(format!(
                "working directory '{}' does not exist",
                workspace_dir.display()
            )),
            _ => BuildError::Io(err),
        })?;

    let mut canonical_inputs = Vec::new();
    let mut seen = BTreeSet::new();
    for source in sources {
        let supplied = PathBuf::from(source);
        let candidate = if supplied.is_absolute() {
            supplied
        } else {
            workspace_root.join(supplied)
        };
        let canonical = candidate.canonicalize().map_err(|_| {
            BuildError::MissingSource(PathBuf::from(format!(
                "ad-hoc source '{source}' was not found"
            )))
        })?;
        if !canonical.is_file() {
            return Err(BuildError::MissingSource(PathBuf::from(format!(
                "ad-hoc source '{source}' is not a regular file"
            ))));
        }
        if seen.insert(canonical.clone()) {
            canonical_inputs.push(canonical);
        }
    }

    let mut canonical_include_dirs = Vec::new();
    let mut seen_includes = BTreeSet::new();
    for include in include_dirs {
        let supplied = PathBuf::from(include);
        let candidate = if supplied.is_absolute() {
            supplied
        } else {
            workspace_root.join(supplied)
        };
        let canonical = candidate.canonicalize().map_err(|_| {
            BuildError::InvalidConfiguration(format!(
                "ad-hoc include directory '{include}' was not found"
            ))
        })?;
        if !canonical.is_dir() {
            return Err(BuildError::InvalidConfiguration(format!(
                "ad-hoc include path '{include}' is not a directory"
            )));
        }
        if seen_includes.insert(canonical.clone()) {
            canonical_include_dirs.push(canonical);
        }
    }

    let language = infer_language(&canonical_inputs)?;
    let first_stem = canonical_inputs
        .first()
        .and_then(|path| path.file_stem())
        .and_then(|stem| stem.to_str())
        .unwrap_or("hello");
    let target_name = sanitize_target_name(first_stem);
    let digest = stable_fingerprint(&workspace_root, &canonical_inputs, &canonical_include_dirs);
    let scratch_root = workspace_root
        .join(".crux")
        .join("run")
        .join(format!("{target_name}-{digest}"));

    for input in &canonical_inputs {
        if input.starts_with(&scratch_root) {
            return Err(BuildError::InvalidConfiguration(format!(
                "ad-hoc source '{}' is inside the Crux scratch directory '{}'",
                input.display(),
                scratch_root.display()
            )));
        }
    }
    for include in &canonical_include_dirs {
        if include.starts_with(&scratch_root) {
            return Err(BuildError::InvalidConfiguration(format!(
                "ad-hoc include directory '{}' is inside the Crux scratch directory '{}'",
                include.display(),
                scratch_root.display()
            )));
        }
    }

    if scratch_root.exists() {
        fs::remove_dir_all(&scratch_root).map_err(BuildError::Io)?;
    }
    fs::create_dir_all(&scratch_root).map_err(BuildError::Io)?;

    let input_root = common_ancestor(&canonical_inputs).ok_or_else(|| {
        BuildError::InvalidConfiguration(
            "ad-hoc sources do not share a common parent directory".to_string(),
        )
    })?;
    let scratch_src_dir = scratch_root.join("src");
    fs::create_dir_all(&scratch_src_dir).map_err(BuildError::Io)?;

    let mut copied_inputs = Vec::new();
    for input in &canonical_inputs {
        let relative = input.strip_prefix(&input_root).map_err(|_| {
            BuildError::SecurityViolation(format!(
                "ad-hoc source '{}' escapes its common source root '{}'",
                input.display(),
                input_root.display()
            ))
        })?;
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(BuildError::SecurityViolation(format!(
                "ad-hoc source '{}' has an unsafe relative path",
                input.display()
            )));
        }
        let destination = scratch_src_dir.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(BuildError::Io)?;
        }
        fs::copy(input, &destination).map_err(BuildError::Io)?;
        copied_inputs.push(destination);
    }

    let sources = DiscoveredSources {
        root_src_dir: scratch_src_dir.clone(),
        source_roots: vec![scratch_src_dir],
        files: copied_inputs
            .iter()
            .filter(|path| is_compilation_unit(path, language))
            .cloned()
            .collect(),
    };
    if sources.files.is_empty() {
        return Err(BuildError::InvalidConfiguration(
            "ad-hoc sources contain no compilation units for the detected language".to_string(),
        ));
    }

    let standard = normalize_standard(default_standard_for_language(language).unwrap_or("c++20"));
    let toolchain =
        detect_project_toolchain(language, Some(&standard)).map_err(map_toolchain_error)?;
    let cmake: CMakeInfo = detect_cmake(Path::new("cmake"))?;

    let config = ProjectConfig {
        name: target_name.clone(),
        version: semver::Version::new(0, 1, 0),
        language: Some(language),
        standard: Some(standard.clone()),
        is_project: true,
        dependencies: Vec::new(),
    };
    let target = AppTarget::executable(&target_name);
    let layout = AppBuildLayout::new(&scratch_root, profile, &target);

    let model = AppBuildModel {
        project_root: scratch_root.clone(),
        config,
        language,
        standard,
        toolchain,
        sources,
        extra_include_dirs: canonical_include_dirs,
        project_include_dirs: Vec::new(),
        dependencies: AppDependencyPlan::default(),
        target,
        profile,
        layout,
        cmake,
    };

    Ok((
        model,
        AdhocRunPlan {
            workspace_root: scratch_root,
            language,
            target_name,
        },
    ))
}

fn infer_language(inputs: &[PathBuf]) -> Result<ProjectLanguage, BuildError> {
    let mut language = None;
    for input in inputs {
        let extension = input
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase);
        let is_header = extension
            .as_deref()
            .is_some_and(|ext| HEADER_EXTENSIONS.contains(&ext));
        let file_language = match extension.as_deref() {
            Some("c") => Some(ProjectLanguage::C),
            Some("cpp") | Some("cc") | Some("cxx") => Some(ProjectLanguage::Cpp),
            _ if is_header => None,
            Some(ext) => {
                return Err(BuildError::InvalidConfiguration(format!(
                    "unsupported ad-hoc source extension '.{ext}' in '{}'; expected .c, .cpp, .cc, .cxx, or a supported header extension",
                    input.display()
                )));
            }
            None => {
                return Err(BuildError::InvalidConfiguration(format!(
                    "ad-hoc source '{}' has no file extension",
                    input.display()
                )));
            }
        };

        if let Some(file_language) = file_language {
            match language {
                None => language = Some(file_language),
                Some(existing) if existing == file_language => {}
                Some(existing) => {
                    return Err(BuildError::InvalidConfiguration(format!(
                        "mixed C and C++ compilation units are not supported in ad-hoc runs ('{}' uses {}, expected {})",
                        input.display(),
                        file_language.display_name(),
                        existing.display_name()
                    )));
                }
            }
        }
    }

    language.ok_or_else(|| {
        BuildError::InvalidConfiguration(
            "ad-hoc sources contain only header files; at least one .c or .cpp file is required"
                .to_string(),
        )
    })
}

fn is_compilation_unit(path: &Path, language: ProjectLanguage) -> bool {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    match language {
        ProjectLanguage::C => extension.as_deref() == Some("c"),
        ProjectLanguage::Cpp => matches!(extension.as_deref(), Some("cpp" | "cc" | "cxx")),
        ProjectLanguage::Other => false,
    }
}

fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    let first = paths.first()?;
    let mut common: Vec<_> = first.components().collect();
    for path in &paths[1..] {
        let components: Vec<_> = path.components().collect();
        let shared = common
            .iter()
            .zip(components.iter())
            .take_while(|(left, right)| left == right)
            .count();
        common.truncate(shared);
    }

    let mut result = PathBuf::new();
    for component in common {
        result.push(component.as_os_str());
    }
    if paths.iter().any(|path| path == &result) {
        result = result.parent()?.to_path_buf();
    }
    if result.as_os_str().is_empty() {
        None
    } else {
        Some(result)
    }
}

fn stable_fingerprint(
    workspace_root: &Path,
    inputs: &[PathBuf],
    include_dirs: &[PathBuf],
) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for path in std::iter::once(workspace_root)
        .chain(inputs.iter().map(PathBuf::as_path))
        .chain(include_dirs.iter().map(PathBuf::as_path))
    {
        for byte in path.to_string_lossy().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn map_toolchain_error(error: ToolchainError) -> BuildError {
    match error {
        ToolchainError::CompilerNotFound {
            language,
            candidate_or_env,
        } => BuildError::ToolchainDetectionFailed(format!(
            "no compatible {} compiler found ({candidate_or_env})",
            language.display_name()
        )),
        ToolchainError::UnsupportedStandard {
            language,
            standard,
            compiler,
            compiler_version,
        } => BuildError::InvalidConfiguration(format!(
            "compiler {compiler} ({compiler_version}) does not support requested {} standard '{standard}'",
            language.display_name()
        )),
        other => BuildError::ToolchainDetectionFailed(other.to_string()),
    }
}
