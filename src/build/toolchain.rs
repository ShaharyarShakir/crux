//! Build toolchain detection and environment inspection.

use crate::project::manifest::{
    ProjectLanguage, default_standard_for_language, is_valid_c_standard, is_valid_cpp_standard,
    normalize_standard,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Information about the detected build toolchain (used by package build engine).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolchainInfo {
    pub cmake_version: String,
    pub c_compiler_version: Option<String>,
    pub cxx_compiler_version: Option<String>,
    pub generator: String,
}

/// Known compiler families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CompilerFamily {
    Gcc,
    Clang,
    AppleClang,
    Msvc,
}

impl fmt::Display for CompilerFamily {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompilerFamily::Gcc => write!(f, "GCC"),
            CompilerFamily::Clang => write!(f, "Clang"),
            CompilerFamily::AppleClang => write!(f, "Apple Clang"),
            CompilerFamily::Msvc => write!(f, "MSVC"),
        }
    }
}

/// Structured information describing an inspected C or C++ compiler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilerInfo {
    pub family: CompilerFamily,
    pub language: ProjectLanguage,
    pub version: String,
    pub target_triple: String,
    pub path: PathBuf,
}

impl fmt::Display for CompilerInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Compiler: {}", self.family)?;
        writeln!(f, "Language: {}", self.language.display_name())?;
        writeln!(f, "Version: {}", self.version)?;
        writeln!(f, "Target: {}", self.target_triple)?;
        write!(f, "Executable: {}", self.path.display())
    }
}

/// Structured platform architecture and operating system information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformInfo {
    pub os: String,
    pub arch: String,
}

impl PlatformInfo {
    /// Returns the current host platform information.
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }
    }
}

impl fmt::Display for PlatformInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.arch, self.os)
    }
}

/// Result of complete toolchain and standard detection for a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectToolchain {
    pub language: ProjectLanguage,
    pub compiler: CompilerInfo,
    pub requested_standard: Option<String>,
    pub standard_supported: bool,
    pub platform: PlatformInfo,
}

/// Errors that can occur during compiler detection and standard capability probing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolchainError {
    CompilerNotFound {
        language: ProjectLanguage,
        candidate_or_env: String,
    },
    CompilerExecutionFailed {
        path: PathBuf,
        reason: String,
    },
    VersionParseError {
        path: PathBuf,
        output: String,
    },
    UnsupportedStandard {
        language: ProjectLanguage,
        standard: String,
        compiler: String,
        compiler_version: String,
    },
    IncompatibleLanguageStandard {
        language: ProjectLanguage,
        standard: String,
    },
}

impl fmt::Display for ToolchainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolchainError::CompilerNotFound {
                language,
                candidate_or_env,
            } => {
                if candidate_or_env.starts_with("CC=") || candidate_or_env.starts_with("CXX=") {
                    write!(
                        f,
                        "error: compiler specified by '{candidate_or_env}' not found or not executable"
                    )
                } else {
                    write!(
                        f,
                        "error: no suitable {} compiler found (tried {candidate_or_env})",
                        language.display_name()
                    )
                }
            }
            ToolchainError::CompilerExecutionFailed { path, reason } => {
                write!(
                    f,
                    "error: failed to execute compiler at '{}': {reason}",
                    path.display()
                )
            }
            ToolchainError::VersionParseError { path, output } => {
                write!(
                    f,
                    "error: failed to parse version from compiler '{}': {output}",
                    path.display()
                )
            }
            ToolchainError::UnsupportedStandard {
                language: _,
                standard,
                compiler,
                compiler_version,
            } => {
                write!(
                    f,
                    "error: compiler '{compiler} {compiler_version}' does not support requested language standard '{standard}'"
                )
            }
            ToolchainError::IncompatibleLanguageStandard { language, standard } => {
                write!(
                    f,
                    "error: language standard '{standard}' is not compatible with language '{}'",
                    language.display_name()
                )
            }
        }
    }
}

impl std::error::Error for ToolchainError {}

/// Resolves a binary command name into an absolute path by searching `PATH`.
pub fn find_in_path(binary: &Path) -> Option<PathBuf> {
    if binary.is_absolute() || binary.components().count() > 1 {
        if binary.is_file() {
            return Some(binary.to_path_buf());
        }
        return None;
    }

    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        {
            let exe = dir.join(format!("{}.exe", binary.display()));
            if exe.is_file() {
                return Some(exe);
            }
        }
    }

    None
}

/// Inspects a compiler executable at the given path, determining family, version, and target triple.
pub fn inspect_compiler(
    path: &Path,
    language: ProjectLanguage,
) -> Result<CompilerInfo, ToolchainError> {
    let resolved_path = find_in_path(path).ok_or_else(|| ToolchainError::CompilerNotFound {
        language,
        candidate_or_env: path.display().to_string(),
    })?;

    let version_output = Command::new(&resolved_path)
        .arg("--version")
        .output()
        .map_err(|e| ToolchainError::CompilerExecutionFailed {
            path: resolved_path.clone(),
            reason: e.to_string(),
        })?;

    if !version_output.status.success() {
        return Err(ToolchainError::CompilerExecutionFailed {
            path: resolved_path,
            reason: format!(
                "exited with status {}",
                version_output.status.code().unwrap_or(-1)
            ),
        });
    }

    let version_str = String::from_utf8_lossy(&version_output.stdout);

    // 1. Determine CompilerFamily
    let lower = version_str.to_ascii_lowercase();
    let family = if lower.contains("apple clang") || lower.contains("apple llvm") {
        CompilerFamily::AppleClang
    } else if lower.contains("clang") {
        CompilerFamily::Clang
    } else if lower.contains("(gcc)")
        || lower.contains("free software foundation")
        || lower.starts_with("gcc")
        || lower.starts_with("g++")
    {
        CompilerFamily::Gcc
    } else if lower.contains("microsoft") || lower.contains("optimizing compiler") {
        CompilerFamily::Msvc
    } else {
        let bin_name = resolved_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if bin_name.contains("clang") {
            CompilerFamily::Clang
        } else if bin_name.contains("cl") {
            CompilerFamily::Msvc
        } else {
            CompilerFamily::Gcc
        }
    };

    // 2. Determine Version (prefer -dumpversion if available)
    let mut detected_version = None;
    if let Ok(dump_out) = Command::new(&resolved_path).arg("-dumpversion").output() {
        if dump_out.status.success() {
            let s = String::from_utf8_lossy(&dump_out.stdout).trim().to_string();
            if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.') {
                detected_version = Some(s);
            }
        }
    }

    let version = match detected_version {
        Some(v) => v,
        None => {
            parse_version_string(&version_str).ok_or_else(|| ToolchainError::VersionParseError {
                path: resolved_path.clone(),
                output: version_str.lines().next().unwrap_or("").to_string(),
            })?
        }
    };

    // 3. Determine Target Triple (prefer -dumpmachine if available)
    let mut detected_target = None;
    if let Ok(dump_out) = Command::new(&resolved_path).arg("-dumpmachine").output() {
        if dump_out.status.success() {
            let s = String::from_utf8_lossy(&dump_out.stdout).trim().to_string();
            if !s.is_empty() {
                detected_target = Some(s);
            }
        }
    }

    let target_triple = detected_target.unwrap_or_else(|| {
        parse_target_triple(&version_str).unwrap_or_else(|| {
            format!(
                "{}-unknown-{}",
                std::env::consts::ARCH,
                std::env::consts::OS
            )
        })
    });

    Ok(CompilerInfo {
        family,
        language,
        version,
        target_triple,
        path: resolved_path,
    })
}

/// Parses the first version-like token from compiler output.
pub fn parse_version_string(output: &str) -> Option<String> {
    for line in output.lines() {
        for token in line.split_whitespace() {
            let trimmed = token.trim_matches(|c: char| !c.is_ascii_digit());
            if trimmed.contains('.') && trimmed.chars().all(|c| c.is_ascii_digit() || c == '.') {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Parses the Target triple from compiler `--version` output.
pub fn parse_target_triple(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(target) = trimmed.strip_prefix("Target:") {
            let val = target.trim();
            if !val.is_empty() {
                return Some(val.to_string());
            }
        }
    }
    None
}

/// Detects the preferred installed compiler for the given language.
pub fn detect_compiler(language: ProjectLanguage) -> Result<CompilerInfo, ToolchainError> {
    detect_compiler_with_custom(language, None)
}

/// Detects the preferred installed compiler for the given language, honoring an optional override.
pub fn detect_compiler_with_custom(
    language: ProjectLanguage,
    custom_bin: Option<&Path>,
) -> Result<CompilerInfo, ToolchainError> {
    if let Some(custom) = custom_bin {
        return inspect_compiler(custom, language);
    }

    let env_var = match language {
        ProjectLanguage::C => "CC",
        ProjectLanguage::Cpp => "CXX",
        ProjectLanguage::Other => {
            return Err(ToolchainError::CompilerNotFound {
                language,
                candidate_or_env: "none (Other language does not use a C/C++ compiler)".to_string(),
            });
        }
    };

    if let Ok(val) = std::env::var(env_var) {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            let path = Path::new(trimmed);
            return inspect_compiler(path, language).map_err(|_| {
                ToolchainError::CompilerNotFound {
                    language,
                    candidate_or_env: format!("{env_var}={trimmed}"),
                }
            });
        }
    }

    let candidates: &[&str] = match language {
        ProjectLanguage::C => &["gcc", "clang"],
        ProjectLanguage::Cpp => &["g++", "clang++"],
        ProjectLanguage::Other => &[],
    };

    for candidate in candidates {
        let candidate_path = Path::new(candidate);
        if let Ok(info) = inspect_compiler(candidate_path, language) {
            return Ok(info);
        }
    }

    Err(ToolchainError::CompilerNotFound {
        language,
        candidate_or_env: candidates.join(", "),
    })
}

/// Maps a normalized language standard to its compiler `-std=` flag and optional fallback flag.
pub fn standard_to_flag(
    language: ProjectLanguage,
    standard: &str,
) -> Result<(&'static str, Option<&'static str>), ToolchainError> {
    match language {
        ProjectLanguage::C => {
            if !is_valid_c_standard(standard) {
                return Err(ToolchainError::IncompatibleLanguageStandard {
                    language,
                    standard: standard.to_string(),
                });
            }
            match standard.to_ascii_lowercase().as_str() {
                "c17" | "c18" => Ok(("c17", Some("c18"))),
                "c23" => Ok(("c23", Some("c2x"))),
                "c11" => Ok(("c11", None)),
                "c99" => Ok(("c99", None)),
                "c89" | "c90" => Ok(("c89", Some("c90"))),
                _ => Ok(("c17", None)),
            }
        }
        ProjectLanguage::Cpp => {
            if !is_valid_cpp_standard(standard) {
                return Err(ToolchainError::IncompatibleLanguageStandard {
                    language,
                    standard: standard.to_string(),
                });
            }
            let normalized = normalize_standard(standard);
            match normalized.as_str() {
                "c++20" => Ok(("c++20", Some("c++2a"))),
                "c++23" => Ok(("c++23", Some("c++2b"))),
                "c++26" => Ok(("c++26", Some("c++2c"))),
                "c++17" => Ok(("c++17", Some("c++1z"))),
                "c++14" => Ok(("c++14", Some("c++1y"))),
                "c++11" => Ok(("c++11", Some("c++0x"))),
                "c++98" | "c++03" => Ok(("c++98", Some("c++03"))),
                _ => Ok(("c++20", None)),
            }
        }
        ProjectLanguage::Other => Err(ToolchainError::IncompatibleLanguageStandard {
            language,
            standard: standard.to_string(),
        }),
    }
}

/// Probes whether a compiler supports a given language standard by compiling a minimal test file.
pub fn probe_standard_support(
    compiler: &CompilerInfo,
    standard: &str,
) -> Result<bool, ToolchainError> {
    let (primary_flag, fallback_flag) = standard_to_flag(compiler.language, standard)?;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let temp_dir = std::env::temp_dir().join(format!("crux_probe_{pid}_{nanos}"));
    std::fs::create_dir_all(&temp_dir).map_err(|e| ToolchainError::CompilerExecutionFailed {
        path: compiler.path.clone(),
        reason: format!("failed to create probe temporary directory: {e}"),
    })?;

    struct TempDirGuard(PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _guard = TempDirGuard(temp_dir.clone());

    let (src_filename, src_content) = match compiler.language {
        ProjectLanguage::C => ("probe.c", "int main(void) { return 0; }\n"),
        ProjectLanguage::Cpp => ("probe.cpp", "int main() { return 0; }\n"),
        ProjectLanguage::Other => {
            return Err(ToolchainError::IncompatibleLanguageStandard {
                language: ProjectLanguage::Other,
                standard: standard.to_string(),
            });
        }
    };

    let src_file = temp_dir.join(src_filename);
    let obj_file = temp_dir.join("probe.o");

    std::fs::write(&src_file, src_content).map_err(|e| {
        ToolchainError::CompilerExecutionFailed {
            path: compiler.path.clone(),
            reason: format!("failed to write probe source file: {e}"),
        }
    })?;

    // 1. Try primary standard flag
    let primary_success = Command::new(&compiler.path)
        .arg(format!("-std={primary_flag}"))
        .arg("-c")
        .arg(&src_file)
        .arg("-o")
        .arg(&obj_file)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if primary_success {
        return Ok(true);
    }

    // 2. Try fallback standard flag if available
    if let Some(fallback) = fallback_flag {
        let fallback_success = Command::new(&compiler.path)
            .arg(format!("-std={fallback}"))
            .arg("-c")
            .arg(&src_file)
            .arg("-o")
            .arg(&obj_file)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if fallback_success {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Detects the project toolchain and verifies compatibility with requested language standard.
pub fn detect_project_toolchain(
    language: ProjectLanguage,
    requested_standard: Option<&str>,
) -> Result<ProjectToolchain, ToolchainError> {
    detect_project_toolchain_with_custom(language, requested_standard, None)
}

/// Detects the project toolchain and verifies compatibility with requested standard, honoring an override.
pub fn detect_project_toolchain_with_custom(
    language: ProjectLanguage,
    requested_standard: Option<&str>,
    custom_bin: Option<&Path>,
) -> Result<ProjectToolchain, ToolchainError> {
    if language == ProjectLanguage::Other {
        return Err(ToolchainError::CompilerNotFound {
            language,
            candidate_or_env: "none (Other projects do not require toolchains)".to_string(),
        });
    }

    let compiler = detect_compiler_with_custom(language, custom_bin)?;
    let platform = PlatformInfo::current();

    let standard = match requested_standard {
        Some(s) => Some(s.to_string()),
        None => default_standard_for_language(language).map(|s| s.to_string()),
    };

    let mut standard_supported = false;
    if let Some(ref std_str) = standard {
        let supported = probe_standard_support(&compiler, std_str)?;
        if !supported {
            return Err(ToolchainError::UnsupportedStandard {
                language,
                standard: std_str.clone(),
                compiler: compiler.family.to_string(),
                compiler_version: compiler.version.clone(),
            });
        }
        standard_supported = true;
    }

    Ok(ProjectToolchain {
        language,
        compiler,
        requested_standard: standard,
        standard_supported,
        platform,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_info_current() {
        let platform = PlatformInfo::current();
        assert!(!platform.os.is_empty());
        assert!(!platform.arch.is_empty());
    }

    #[test]
    fn test_parse_version_string() {
        let gcc_out = "gcc (GCC) 15.3.0\nCopyright (C) 2025 Free Software Foundation, Inc.";
        assert_eq!(parse_version_string(gcc_out), Some("15.3.0".to_string()));

        let clang_out = "clang version 21.1.8\nTarget: x86_64-unknown-linux-gnu";
        assert_eq!(parse_version_string(clang_out), Some("21.1.8".to_string()));

        let empty = "no numbers here";
        assert_eq!(parse_version_string(empty), None);
    }

    #[test]
    fn test_parse_target_triple() {
        let clang_out =
            "clang version 21.1.8\nTarget: x86_64-unknown-linux-gnu\nThread model: posix";
        assert_eq!(
            parse_target_triple(clang_out),
            Some("x86_64-unknown-linux-gnu".to_string())
        );

        let gcc_out = "gcc (GCC) 15.3.0\nCopyright ...";
        assert_eq!(parse_target_triple(gcc_out), None);
    }

    #[test]
    fn test_standard_to_flag_c() {
        assert_eq!(
            standard_to_flag(ProjectLanguage::C, "c17").unwrap(),
            ("c17", Some("c18"))
        );
        assert_eq!(
            standard_to_flag(ProjectLanguage::C, "c23").unwrap(),
            ("c23", Some("c2x"))
        );
        assert!(standard_to_flag(ProjectLanguage::C, "c++20").is_err());
    }

    #[test]
    fn test_standard_to_flag_cpp() {
        assert_eq!(
            standard_to_flag(ProjectLanguage::Cpp, "c++20").unwrap(),
            ("c++20", Some("c++2a"))
        );
        assert_eq!(
            standard_to_flag(ProjectLanguage::Cpp, "c++23").unwrap(),
            ("c++23", Some("c++2b"))
        );
        assert!(standard_to_flag(ProjectLanguage::Cpp, "c17").is_err());
    }

    #[test]
    fn test_detect_compiler_c_or_cpp() {
        // Detects whatever compiler is installed on the host
        if let Ok(info) = detect_compiler(ProjectLanguage::C) {
            assert!(info.path.is_file());
            assert!(!info.version.is_empty());
        }
        if let Ok(info) = detect_compiler(ProjectLanguage::Cpp) {
            assert!(info.path.is_file());
            assert!(!info.version.is_empty());
        }
    }

    #[test]
    fn test_compiler_info_display() {
        let info = CompilerInfo {
            family: CompilerFamily::Gcc,
            language: ProjectLanguage::Cpp,
            version: "15.3.0".to_string(),
            target_triple: "x86_64-unknown-linux-gnu".to_string(),
            path: PathBuf::from("/usr/bin/g++"),
        };
        let disp = info.to_string();
        assert!(disp.contains("Compiler: GCC"));
        assert!(disp.contains("Language: C++"));
        assert!(disp.contains("Version: 15.3.0"));
        assert!(disp.contains("Target: x86_64-unknown-linux-gnu"));
        assert!(disp.contains("Executable: /usr/bin/g++"));
    }

    #[test]
    fn test_detect_compiler_not_found_for_other() {
        let result = detect_compiler(ProjectLanguage::Other);
        assert!(matches!(
            result,
            Err(ToolchainError::CompilerNotFound { .. })
        ));
    }
}
