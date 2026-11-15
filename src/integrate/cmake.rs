//! CMake consumer script generation and atomic file emission.

use super::IntegrationError;
use super::validation::ValidatedPackageInstallation;
use crate::build::BuildProfile;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Default relative output path for the CMake integration script.
pub const DEFAULT_CMAKE_INTEGRATION_SUBPATH: &str = ".crux/cmake/CruxDependencies.cmake";

/// Result of generating the CMake integration file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationResult {
    pub integrated_packages: Vec<ValidatedPackageInstallation>,
    pub output_path: PathBuf,
    pub content: String,
}

/// Escapes a path string for safe inclusion in CMake scripts.
pub fn escape_cmake_path(path: &str) -> Result<String, IntegrationError> {
    if path.contains('\n') || path.contains('\r') || path.contains('\0') {
        return Err(IntegrationError::InvalidPath(format!(
            "path contains forbidden newline or null characters: '{path}'"
        )));
    }

    // Canonicalize Windows backslashes to forward slashes for CMake
    let normalized = path.replace('\\', "/");
    let mut escaped = String::with_capacity(normalized.len() + 10);
    for ch in normalized.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '$' => escaped.push_str("\\$"),
            ';' => escaped.push_str("\\;"),
            c => escaped.push(c),
        }
    }
    Ok(escaped)
}

/// Sanitizes a package name for use as a valid CMake variable identifier.
pub fn sanitize_cmake_identifier(name: &str) -> String {
    let mut ident = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            ident.push(ch.to_ascii_uppercase());
        } else {
            ident.push('_');
        }
    }
    if ident.is_empty() {
        "CRUX_PKG".to_string()
    } else {
        ident
    }
}

/// Generates the complete, deterministic contents of `CruxDependencies.cmake`.
pub fn generate_cmake_dependencies(
    _project_root: &Path,
    packages: &[ValidatedPackageInstallation],
) -> Result<String, IntegrationError> {
    let mut out = String::new();

    out.push_str(
        "# =============================================================================\n",
    );
    out.push_str("# Generated automatically by Crux. DO NOT EDIT MANUALLY.\n");
    out.push_str("# Subsystem: Crux CMake Consumer Integration\n");
    out.push_str("# Target: .crux/cmake/CruxDependencies.cmake\n");
    out.push_str(
        "# =============================================================================\n\n",
    );

    out.push_str("cmake_minimum_required(VERSION 3.20)\n\n");

    out.push_str("# Guard against duplicate inclusions\n");
    out.push_str("if(DEFINED _CRUX_DEPENDENCIES_INCLUDED)\n");
    out.push_str("    return()\n");
    out.push_str("endif()\n");
    out.push_str("set(_CRUX_DEPENDENCIES_INCLUDED TRUE)\n\n");

    out.push_str("# Determine the Crux project root directory relative to this file\n");
    out.push_str("get_filename_component(_CRUX_PROJECT_DIR \"${CMAKE_CURRENT_LIST_DIR}/../..\" ABSOLUTE)\n\n");

    if packages.is_empty() {
        out.push_str("# Note: No Crux dependencies are currently configured or installed.\n");
        return Ok(out);
    }

    let mut sorted_packages = packages.to_vec();
    sorted_packages.sort_by(|a, b| a.package_id.name().cmp(b.package_id.name()));

    let mut prefix_vars = Vec::new();

    for pkg in &sorted_packages {
        let pkg_name = pkg.package_id.name().as_str();
        let pkg_version = pkg.package_id.version().to_string();
        let safe_ident = sanitize_cmake_identifier(pkg_name);
        let prefix_var = format!("_CRUX_{safe_ident}_PREFIX");

        let rel_path = format!(
            ".crux/installed/{}/{}/{}",
            pkg_name,
            pkg_version,
            pkg.fingerprint.hex_digest()
        );
        let escaped_rel_path = escape_cmake_path(&rel_path)?;

        out.push_str(&format!("# Dependency: {pkg_name} ({pkg_version})\n"));
        out.push_str(&format!(
            "set({prefix_var} \"${{_CRUX_PROJECT_DIR}}/{escaped_rel_path}\")\n"
        ));
        out.push_str(&format!("if(NOT IS_DIRECTORY \"${{{prefix_var}}}\")\n"));
        out.push_str(&format!(
            "    message(FATAL_ERROR \"Crux dependency '{pkg_name}' ({pkg_version}) not found at expected path: '${{{prefix_var}}}'. Run 'crux install' to restore installed dependencies.\")\n"
        ));
        out.push_str("endif()\n\n");

        prefix_vars.push(prefix_var.clone());

        out.push_str(&format!(
            "if(IS_DIRECTORY \"${{{prefix_var}}}/share/cmake/Modules\")\n"
        ));
        out.push_str(&format!(
            "    list(APPEND _CRUX_ORDERED_MODULE_PATHS \"${{{prefix_var}}}/share/cmake/Modules\")\n"
        ));
        out.push_str("endif()\n");
        out.push_str(&format!("if(IS_DIRECTORY \"${{{prefix_var}}}/cmake\")\n"));
        out.push_str(&format!(
            "    list(APPEND _CRUX_ORDERED_MODULE_PATHS \"${{{prefix_var}}}/cmake\")\n"
        ));
        out.push_str("endif()\n\n");
    }

    out.push_str("# Prepend Crux package prefixes so that Crux dependencies take priority\n");
    out.push_str("# over system and environment search paths while preserving existing entries.\n");
    out.push_str("set(_CRUX_ORDERED_PREFIXES\n");
    for var in &prefix_vars {
        out.push_str(&format!("    \"${{{var}}}\"\n"));
    }
    out.push_str(")\n\n");

    out.push_str("set(_CRUX_FINAL_PREFIX_PATH \"\")\n");
    out.push_str("foreach(_crux_p IN LISTS _CRUX_ORDERED_PREFIXES)\n");
    out.push_str("    if(NOT \"${_crux_p}\" IN_LIST _CRUX_FINAL_PREFIX_PATH)\n");
    out.push_str("        list(APPEND _CRUX_FINAL_PREFIX_PATH \"${_crux_p}\")\n");
    out.push_str("    endif()\n");
    out.push_str("endforeach()\n");
    out.push_str("foreach(_crux_p IN LISTS CMAKE_PREFIX_PATH)\n");
    out.push_str("    if(NOT \"${_crux_p}\" IN_LIST _CRUX_FINAL_PREFIX_PATH)\n");
    out.push_str("        list(APPEND _CRUX_FINAL_PREFIX_PATH \"${_crux_p}\")\n");
    out.push_str("    endif()\n");
    out.push_str("endforeach()\n");
    out.push_str("set(CMAKE_PREFIX_PATH \"${_CRUX_FINAL_PREFIX_PATH}\")\n\n");

    out.push_str("# Update CMAKE_MODULE_PATH if any dependency provides CMake modules\n");
    out.push_str("if(DEFINED _CRUX_ORDERED_MODULE_PATHS)\n");
    out.push_str("    set(_CRUX_FINAL_MODULE_PATH \"\")\n");
    out.push_str("    foreach(_crux_m IN LISTS _CRUX_ORDERED_MODULE_PATHS)\n");
    out.push_str("        if(NOT \"${_crux_m}\" IN_LIST _CRUX_FINAL_MODULE_PATH)\n");
    out.push_str("            list(APPEND _CRUX_FINAL_MODULE_PATH \"${_crux_m}\")\n");
    out.push_str("        endif()\n");
    out.push_str("    endforeach()\n");
    out.push_str("    foreach(_crux_m IN LISTS CMAKE_MODULE_PATH)\n");
    out.push_str("        if(NOT \"${_crux_m}\" IN_LIST _CRUX_FINAL_MODULE_PATH)\n");
    out.push_str("            list(APPEND _CRUX_FINAL_MODULE_PATH \"${_crux_m}\")\n");
    out.push_str("        endif()\n");
    out.push_str("    endforeach()\n");
    out.push_str("    set(CMAKE_MODULE_PATH \"${_CRUX_FINAL_MODULE_PATH}\")\n");
    out.push_str("endif()\n");

    Ok(out)
}

/// Atomically writes the generated CMake integration script to disk.
pub fn write_cmake_dependencies_atomically(
    output_path: &Path,
    content: &str,
) -> Result<(), IntegrationError> {
    let parent = output_path.parent().ok_or_else(|| {
        IntegrationError::InvalidPath(format!(
            "output path '{}' has no parent directory",
            output_path.display()
        ))
    })?;

    fs::create_dir_all(parent).map_err(IntegrationError::Io)?;

    let nonce: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let tmp_filename = format!(
        "{}.tmp.{}",
        output_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("CruxDependencies.cmake"),
        nonce
    );
    let tmp_path = parent.join(tmp_filename);

    let mut file = fs::File::create(&tmp_path).map_err(IntegrationError::Io)?;
    file.write_all(content.as_bytes())
        .map_err(IntegrationError::Io)?;
    file.flush().map_err(IntegrationError::Io)?;
    file.sync_all().map_err(IntegrationError::Io)?;
    drop(file);

    fs::rename(&tmp_path, output_path).map_err(IntegrationError::Io)?;
    Ok(())
}

/// High-level generator managing CMake consumer integration for Crux projects.
#[derive(Debug, Clone)]
pub struct CMakeIntegrationGenerator {
    project_root: PathBuf,
    profile: BuildProfile,
    output_path: PathBuf,
}

impl CMakeIntegrationGenerator {
    /// Creates a new generator for the given project root directory.
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        let root = project_root.into();
        let output_path = root
            .join(".crux")
            .join("cmake")
            .join("CruxDependencies.cmake");
        Self {
            project_root: root,
            profile: BuildProfile::Release,
            output_path,
        }
    }

    /// Sets the target build profile.
    pub fn with_profile(mut self, profile: BuildProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Sets a custom output path for the generated CMake file.
    pub fn with_output_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.output_path = path.into();
        self
    }

    /// Returns the project root directory.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Returns the target output path.
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    /// Returns the build profile.
    pub fn profile(&self) -> BuildProfile {
        self.profile
    }

    /// Generates and atomically writes the CMake integration file for the given validated packages.
    pub fn integrate_packages(
        &self,
        packages: &[ValidatedPackageInstallation],
    ) -> Result<IntegrationResult, IntegrationError> {
        let content = generate_cmake_dependencies(&self.project_root, packages)?;
        write_cmake_dependencies_atomically(&self.output_path, &content)?;
        Ok(IntegrationResult {
            integrated_packages: packages.to_vec(),
            output_path: self.output_path.clone(),
            content,
        })
    }
}
