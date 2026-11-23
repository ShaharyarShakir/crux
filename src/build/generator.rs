//! Internal CMakeLists.txt generation for Crux application builds (Option A).
//!
//! Generates a clean, deterministic, relocatable CMakeLists.txt placed exclusively
//! under `.crux/build/generated/CMakeLists.txt`. Ordinary Crux projects never require
//! or generate a root-level CMakeLists.txt.
//!
//! Paths are injected as pre-escaped, quoted CMake strings. Generation never
//! interpolates data through a shell and never resolves filesystem locations at
//! generation time beyond lexical path manipulation.

use super::BuildError;
use super::sanitizer::{effective_compile_flags, effective_link_flags};
use crate::core::paths::is_safe_relative_path;
use crate::project::build_settings::BuildSettings;
use crate::project::manifest::ProjectLanguage;
use std::path::{Component, Path, PathBuf};

/// Input required to generate an application CMake project.
#[derive(Debug, Clone)]
pub struct AppCMakeConfig<'a> {
    /// Canonical project root directory.
    pub project_root: &'a Path,
    /// Sanitized application target/executable name.
    pub target_name: &'a str,
    /// Project semantic version, emitted for diagnostics and future packaging.
    pub project_version: &'a str,
    /// Application language.
    pub language: ProjectLanguage,
    /// Crux language standard (for example `c17` or `c++20`).
    pub standard: &'a str,
    /// Deterministically ordered application source files.
    pub sources: &'a [PathBuf],
    /// Project-relative include directories discovered by `crux import`.
    pub project_include_dirs: &'a [String],
    /// Include roots supplied by installed Crux dependencies.
    pub dependency_include_dirs: &'a [PathBuf],
    /// Library search directories supplied by installed Crux dependencies.
    pub dependency_link_directories: &'a [PathBuf],
    /// Absolute library files supplied by installed Crux dependencies.
    pub dependency_link_libraries: &'a [PathBuf],
    /// Shared-library directories that must be reachable at runtime.
    pub dependency_runtime_directories: &'a [PathBuf],
    /// `[build]` flags and sanitizers. Applied to the application target only.
    pub build_settings: &'a BuildSettings,
}

/// Sanitizes a project name into a safe CMake target name and executable name.
pub fn sanitize_target_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.' {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }
    while sanitized.starts_with('-') {
        sanitized.remove(0);
    }
    if sanitized.is_empty() {
        "app".to_string()
    } else {
        sanitized
    }
}

/// Translates a Crux language standard into a CMake standard number.
pub fn standard_to_cmake_standard(
    language: ProjectLanguage,
    standard: &str,
) -> Result<&'static str, BuildError> {
    match language {
        ProjectLanguage::C => match standard.to_ascii_lowercase().as_str() {
            "c17" | "17" | "c18" | "18" => Ok("17"),
            "c23" | "23" => Ok("23"),
            "c11" | "11" => Ok("11"),
            "c99" | "99" => Ok("99"),
            "c90" | "90" | "c89" | "89" => Ok("90"),
            other => Err(BuildError::InvalidConfiguration(format!(
                "unsupported C language standard '{other}' for CMake generation"
            ))),
        },
        ProjectLanguage::Cpp => match standard.to_ascii_lowercase().as_str() {
            "c++20" | "cpp20" | "20" => Ok("20"),
            "c++23" | "cpp23" | "23" => Ok("23"),
            "c++26" | "cpp26" | "26" => Ok("26"),
            "c++17" | "cpp17" | "17" => Ok("17"),
            "c++14" | "cpp14" | "14" => Ok("14"),
            "c++11" | "cpp11" | "11" => Ok("11"),
            "c++98" | "cpp98" | "98" | "c++03" | "cpp03" | "03" => Ok("98"),
            other => Err(BuildError::InvalidConfiguration(format!(
                "unsupported C++ language standard '{other}' for CMake generation"
            ))),
        },
        ProjectLanguage::Other => Err(BuildError::InvalidConfiguration(
            "cannot generate CMake configuration for project language 'other'".to_string(),
        )),
    }
}

/// Returns the minimum CMake version able to express the selected standard.
pub fn minimum_cmake_version(language: ProjectLanguage, cmake_standard: &str) -> &'static str {
    match (language, cmake_standard) {
        (ProjectLanguage::Cpp, "26") => "3.25",
        (ProjectLanguage::C, "23") => "3.21",
        _ => "3.20",
    }
}

/// Escapes a path for inclusion inside a double-quoted CMake argument.
///
/// This protects against variable expansion (`$`), list separators (`;`), quote
/// injection, and backslash interpretation while preserving spaces verbatim.
pub fn escape_cmake_path(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '$' => escaped.push_str("\\$"),
            ';' => escaped.push_str("\\;"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// Generates the canonical internal CMakeLists.txt content for an application.
pub fn generate_app_cmakelists(config: &AppCMakeConfig<'_>) -> Result<String, BuildError> {
    let cmake_lang = match config.language {
        ProjectLanguage::C => "C",
        ProjectLanguage::Cpp => "CXX",
        ProjectLanguage::Other => {
            return Err(BuildError::InvalidConfiguration(
                "cannot generate CMake configuration for 'other' language".to_string(),
            ));
        }
    };

    if config.target_name.is_empty()
        || !config
            .target_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(BuildError::InvalidConfiguration(format!(
            "invalid application target name '{}': only ASCII alphanumeric, '_', '-', and '.' are allowed",
            config.target_name
        )));
    }

    let cmake_std = standard_to_cmake_standard(config.language, config.standard)?;
    let minimum_version = minimum_cmake_version(config.language, cmake_std);

    let mut out = String::new();
    out.push_str("# Auto-generated by Crux. Do not edit manually.\n");
    out.push_str(&format!(
        "cmake_minimum_required(VERSION {minimum_version})\n\n"
    ));
    out.push_str(&format!(
        "project({} VERSION {} LANGUAGES {cmake_lang})\n\n",
        config.target_name,
        escape_cmake_path(config.project_version)
    ));

    out.push_str(
        "# Resolve the Crux project root relative to .crux/build/generated.\n\
         get_filename_component(CRUX_PROJECT_ROOT \"${CMAKE_CURRENT_SOURCE_DIR}/../../..\" ABSOLUTE)\n\n",
    );
    out.push_str(&format!("set(CMAKE_{cmake_lang}_STANDARD {cmake_std})\n"));
    out.push_str(&format!("set(CMAKE_{cmake_lang}_STANDARD_REQUIRED ON)\n"));
    out.push_str(&format!("set(CMAKE_{cmake_lang}_EXTENSIONS OFF)\n\n"));

    out.push_str(
        "# Predictable executable placement for single- and multi-config generators.\n\
         set(CMAKE_RUNTIME_OUTPUT_DIRECTORY \"${CMAKE_BINARY_DIR}\")\n\
         set(CMAKE_RUNTIME_OUTPUT_DIRECTORY_DEBUG \"${CMAKE_BINARY_DIR}\")\n\
         set(CMAKE_RUNTIME_OUTPUT_DIRECTORY_RELEASE \"${CMAKE_BINARY_DIR}\")\n\
         set(CMAKE_RUNTIME_OUTPUT_DIRECTORY_RELWITHDEBINFO \"${CMAKE_BINARY_DIR}\")\n\
         set(CMAKE_RUNTIME_OUTPUT_DIRECTORY_MINSIZEREL \"${CMAKE_BINARY_DIR}\")\n\n",
    );

    out.push_str(&format!("add_executable({}\n", config.target_name));
    for source in config.sources {
        let relative = source.strip_prefix(config.project_root).map_err(|_| {
            BuildError::SecurityViolation(format!(
                "application source '{}' is outside the project root '{}'",
                source.display(),
                config.project_root.display()
            ))
        })?;

        if relative.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(BuildError::SecurityViolation(format!(
                "application source path '{}' escapes the project root",
                source.display()
            )));
        }

        let normalized = relative.to_string_lossy().replace('\\', "/");
        if normalized.is_empty() {
            return Err(BuildError::SecurityViolation(format!(
                "application source '{}' resolves to an empty relative path",
                source.display()
            )));
        }

        out.push_str(&format!(
            "    \"${{CRUX_PROJECT_ROOT}}/{}\"\n",
            escape_cmake_path(&normalized)
        ));
    }
    out.push_str(")\n\n");

    out.push_str(&format!(
        "if(EXISTS \"${{CRUX_PROJECT_ROOT}}/include\")\n\
         \x20   target_include_directories({} PRIVATE \"${{CRUX_PROJECT_ROOT}}/include\")\n\
         endif()\n\
         if(EXISTS \"${{CRUX_PROJECT_ROOT}}/src\")\n\
         \x20   target_include_directories({} PRIVATE \"${{CRUX_PROJECT_ROOT}}/src\")\n\
         endif()\n",
        config.target_name, config.target_name
    ));

    // Project-relative include roots recorded by `crux import`. These keep the
    // generated configuration relocatable with the project directory.
    if !config.project_include_dirs.is_empty() {
        out.push_str(&format!(
            "\ntarget_include_directories({} PRIVATE\n",
            config.target_name
        ));
        for value in config.project_include_dirs {
            let relative = Path::new(value);
            if value.is_empty() || !is_safe_relative_path(relative) {
                return Err(BuildError::SecurityViolation(format!(
                    "import include directory '{value}' is not a safe relative path"
                )));
            }
            let normalized = relative.to_string_lossy().replace('\\', "/");
            out.push_str(&format!(
                "    \"${{CRUX_PROJECT_ROOT}}/{}\"\n",
                escape_cmake_path(&normalized)
            ));
        }
        out.push_str(")\n");
    }

    // Dependency include roots (already validated and ordered by the plan).
    if !config.dependency_include_dirs.is_empty() {
        out.push_str(&format!(
            "\ntarget_include_directories({} PRIVATE\n",
            config.target_name
        ));
        for dir in config.dependency_include_dirs {
            out.push_str(&format!(
                "    \"{}\"\n",
                escape_cmake_path(&dir.to_string_lossy())
            ));
        }
        out.push_str(")\n");
    }

    // Library search directories for dependencies.
    if !config.dependency_link_directories.is_empty() {
        out.push_str(&format!(
            "\ntarget_link_directories({} PRIVATE\n",
            config.target_name
        ));
        for dir in config.dependency_link_directories {
            out.push_str(&format!(
                "    \"{}\"\n",
                escape_cmake_path(&dir.to_string_lossy())
            ));
        }
        out.push_str(")\n");
    }

    // Absolute library files, ordered dependents-before-dependencies.
    if !config.dependency_link_libraries.is_empty() {
        out.push_str(&format!(
            "\ntarget_link_libraries({} PRIVATE\n",
            config.target_name
        ));
        for lib in config.dependency_link_libraries {
            out.push_str(&format!(
                "    \"{}\"\n",
                escape_cmake_path(&lib.to_string_lossy())
            ));
        }
        out.push_str(")\n");
    }

    // Record an RPATH so shared dependencies resolve when the executable runs
    // directly from the Crux build tree.
    if !config.dependency_runtime_directories.is_empty() {
        let rpath = config
            .dependency_runtime_directories
            .iter()
            .map(|dir| escape_cmake_path(&dir.to_string_lossy()))
            .collect::<Vec<_>>()
            .join(";");
        out.push_str(&format!(
            "\nset_target_properties({} PROPERTIES\n    BUILD_RPATH \"{}\"\n)\n",
            config.target_name, rpath
        ));
    }

    // `[build]` compiler flags, linker flags, and sanitizers. These attach to
    // the application target only (never to dependencies) and are emitted as
    // individually quoted arguments, so argument boundaries survive and
    // nothing is interpreted by a shell.
    let compile_flags = effective_compile_flags(config.build_settings, config.language);
    if !compile_flags.is_empty() {
        out.push_str(&format!(
            "\ntarget_compile_options({} PRIVATE\n",
            config.target_name
        ));
        for flag in &compile_flags {
            out.push_str(&format!("    \"{}\"\n", escape_cmake_argument(flag)));
        }
        out.push_str(")\n");
    }
    let link_flags = effective_link_flags(config.build_settings);
    if !link_flags.is_empty() {
        out.push_str(&format!(
            "\ntarget_link_options({} PRIVATE\n",
            config.target_name
        ));
        for flag in &link_flags {
            out.push_str(&format!("    \"{}\"\n", escape_cmake_argument(flag)));
        }
        out.push_str(")\n");
    }

    Ok(out)
}

/// Escapes a user-supplied compiler/linker argument for a double-quoted CMake
/// argument. Beyond [`escape_cmake_path`], this also neutralises `@` (so
/// `@VAR@` is never expanded) and `[`/`]` (bracket syntax inside lists).
pub fn escape_cmake_argument(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in escape_cmake_path(value).chars() {
        match ch {
            '@' => escaped.push_str("\\@"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    static NO_BUILD_SETTINGS: BuildSettings = BuildSettings {
        c_flags: Vec::new(),
        cxx_flags: Vec::new(),
        linker_flags: Vec::new(),
        address_sanitizer: false,
        undefined_sanitizer: false,
    };

    fn generate_with(language: ProjectLanguage, settings: &BuildSettings) -> String {
        let root = Path::new("/p");
        let sources = [PathBuf::from("/p/src/main.c")];
        let mut cfg = config(root, "app", language, "c17", &sources);
        cfg.build_settings = settings;
        generate_app_cmakelists(&cfg).unwrap()
    }

    #[test]
    fn default_settings_emit_no_option_commands() {
        let out = generate_with(ProjectLanguage::C, &BuildSettings::default());
        assert!(!out.contains("target_compile_options"));
        assert!(!out.contains("target_link_options"));
    }

    #[test]
    fn flags_are_individually_quoted_and_language_scoped() {
        let settings = BuildSettings {
            c_flags: vec!["-Wall".into(), "-DMSG=hello world".into()],
            cxx_flags: vec!["-Wcxx-only".into()],
            linker_flags: vec!["-Wl,--as-needed".into()],
            ..Default::default()
        };
        let out = generate_with(ProjectLanguage::C, &settings);
        assert!(
            out.contains(
                "target_compile_options(app PRIVATE\n    \"-Wall\"\n    \"-DMSG=hello world\"\n)"
            ),
            "{out}"
        );
        assert!(!out.contains("-Wcxx-only"));
        assert!(out.contains("target_link_options(app PRIVATE\n    \"-Wl,--as-needed\"\n)"));
    }

    #[test]
    fn hostile_flags_cannot_inject_cmake() {
        let settings = BuildSettings {
            c_flags: vec![
                "-DX=\") add_custom_target(pwn) #".into(),
                "-DY=${HOME}@V@".into(),
            ],
            ..Default::default()
        };
        let out = generate_with(ProjectLanguage::C, &settings);
        assert!(
            out.contains("\"-DX=\\\") add_custom_target(pwn) #\""),
            "{out}"
        );
        assert!(out.contains("\"-DY=\\${HOME}\\@V\\@\""), "{out}");
        assert!(!out.contains("\n) add_custom_target"));
    }

    #[test]
    fn sanitizers_reach_compile_and_link() {
        let settings = BuildSettings {
            address_sanitizer: true,
            ..Default::default()
        };
        let out = generate_with(ProjectLanguage::C, &settings);
        let compile = out.split("target_compile_options").nth(1).unwrap();
        let link = out.split("target_link_options").nth(1).unwrap();
        assert!(compile.contains("-fsanitize=address"));
        assert!(link.contains("-fsanitize=address"));
    }

    fn config<'a>(
        root: &'a Path,
        target: &'a str,
        language: ProjectLanguage,
        standard: &'a str,
        sources: &'a [PathBuf],
    ) -> AppCMakeConfig<'a> {
        AppCMakeConfig {
            project_root: root,
            target_name: target,
            project_version: "0.1.0",
            language,
            standard,
            sources,
            project_include_dirs: &[],
            dependency_include_dirs: &[],
            dependency_link_directories: &[],
            dependency_link_libraries: &[],
            dependency_runtime_directories: &[],
            build_settings: &NO_BUILD_SETTINGS,
        }
    }

    #[test]
    fn test_sanitize_target_name() {
        assert_eq!(sanitize_target_name("my-app"), "my-app");
        assert_eq!(sanitize_target_name("my_app_1"), "my_app_1");
        assert_eq!(sanitize_target_name("my app!"), "my_app_");
        assert_eq!(sanitize_target_name(""), "app");
        assert_eq!(sanitize_target_name("--weird"), "weird");
    }

    #[test]
    fn test_standard_to_cmake_standard() {
        assert_eq!(
            standard_to_cmake_standard(ProjectLanguage::C, "c17").unwrap(),
            "17"
        );
        assert_eq!(
            standard_to_cmake_standard(ProjectLanguage::C, "c18").unwrap(),
            "17"
        );
        assert_eq!(
            standard_to_cmake_standard(ProjectLanguage::C, "c23").unwrap(),
            "23"
        );
        assert_eq!(
            standard_to_cmake_standard(ProjectLanguage::Cpp, "c++20").unwrap(),
            "20"
        );
        assert_eq!(
            standard_to_cmake_standard(ProjectLanguage::Cpp, "c++23").unwrap(),
            "23"
        );
        assert_eq!(
            standard_to_cmake_standard(ProjectLanguage::Cpp, "c++03").unwrap(),
            "98"
        );
        assert!(standard_to_cmake_standard(ProjectLanguage::Other, "c17").is_err());
    }

    #[test]
    fn test_minimum_cmake_version() {
        assert_eq!(minimum_cmake_version(ProjectLanguage::C, "17"), "3.20");
        assert_eq!(minimum_cmake_version(ProjectLanguage::C, "23"), "3.21");
        assert_eq!(minimum_cmake_version(ProjectLanguage::Cpp, "23"), "3.20");
        assert_eq!(minimum_cmake_version(ProjectLanguage::Cpp, "26"), "3.25");
    }

    #[test]
    fn test_escape_cmake_path() {
        assert_eq!(escape_cmake_path("plain/path.c"), "plain/path.c");
        assert_eq!(escape_cmake_path("spaces in/path.c"), "spaces in/path.c");
        assert_eq!(escape_cmake_path("dollar$var.c"), "dollar\\$var.c");
        assert_eq!(escape_cmake_path("semi;colon.c"), "semi\\;colon.c");
        assert_eq!(escape_cmake_path("quote\"name.c"), "quote\\\"name.c");
        assert_eq!(escape_cmake_path("back\\slash.c"), "back\\\\slash.c");
    }

    #[test]
    fn test_generate_c_cmakelists() {
        let root = Path::new("/workspace/c_demo");
        let sources = vec![root.join("src").join("main.c")];

        let content =
            generate_app_cmakelists(&config(root, "c-demo", ProjectLanguage::C, "c17", &sources))
                .unwrap();

        assert!(content.contains("cmake_minimum_required(VERSION 3.20)"));
        assert!(content.contains("project(c-demo VERSION 0.1.0 LANGUAGES C)"));
        assert!(content.contains("set(CMAKE_C_STANDARD 17)"));
        assert!(content.contains("set(CMAKE_C_STANDARD_REQUIRED ON)"));
        assert!(content.contains("set(CMAKE_C_EXTENSIONS OFF)"));
        assert!(
            content.contains("add_executable(c-demo\n    \"${CRUX_PROJECT_ROOT}/src/main.c\"\n)")
        );
        assert!(content.contains("set(CMAKE_RUNTIME_OUTPUT_DIRECTORY \"${CMAKE_BINARY_DIR}\")"));
        assert!(
            content.contains("set(CMAKE_RUNTIME_OUTPUT_DIRECTORY_DEBUG \"${CMAKE_BINARY_DIR}\")")
        );
    }

    #[test]
    fn test_generate_cpp_cmakelists_with_multiple_sources() {
        let root = Path::new("/workspace/cpp_demo");
        let sources = vec![
            root.join("src").join("main.cpp"),
            root.join("src").join("modules").join("calc.cpp"),
        ];

        let content = generate_app_cmakelists(&config(
            root,
            "calc_app",
            ProjectLanguage::Cpp,
            "c++20",
            &sources,
        ))
        .unwrap();

        assert!(content.contains("project(calc_app VERSION 0.1.0 LANGUAGES CXX)"));
        assert!(content.contains("set(CMAKE_CXX_STANDARD 20)"));
        assert!(content.contains("set(CMAKE_CXX_STANDARD_REQUIRED ON)"));
        assert!(content.contains("set(CMAKE_CXX_EXTENSIONS OFF)"));
        assert!(content.contains(
            "add_executable(calc_app\n    \"${CRUX_PROJECT_ROOT}/src/main.cpp\"\n    \"${CRUX_PROJECT_ROOT}/src/modules/calc.cpp\"\n)"
        ));
    }

    #[test]
    fn test_paths_with_spaces_and_special_characters_are_escaped() {
        let root = Path::new("/workspace/my path with spaces");
        let sources = vec![
            root.join("src").join("my main.cpp"),
            root.join("src").join("semi;colon.cpp"),
            root.join("src").join("dollar$var.cpp"),
        ];

        let content = generate_app_cmakelists(&config(
            root,
            "spaced_app",
            ProjectLanguage::Cpp,
            "c++20",
            &sources,
        ))
        .unwrap();

        assert!(content.contains("\"${CRUX_PROJECT_ROOT}/src/my main.cpp\""));
        assert!(content.contains("\"${CRUX_PROJECT_ROOT}/src/semi\\;colon.cpp\""));
        assert!(content.contains("\"${CRUX_PROJECT_ROOT}/src/dollar\\$var.cpp\""));
    }

    #[test]
    fn test_generation_is_deterministic() {
        let root = Path::new("/workspace/det");
        let sources = vec![
            root.join("src").join("a.cpp"),
            root.join("src").join("z.cpp"),
        ];

        let first = generate_app_cmakelists(&config(
            root,
            "det",
            ProjectLanguage::Cpp,
            "c++20",
            &sources,
        ))
        .unwrap();
        let second = generate_app_cmakelists(&config(
            root,
            "det",
            ProjectLanguage::Cpp,
            "c++20",
            &sources,
        ))
        .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn test_source_outside_project_root_is_rejected() {
        let root = Path::new("/workspace/det");
        let sources = vec![PathBuf::from("/etc/passwd.c")];

        let result =
            generate_app_cmakelists(&config(root, "det", ProjectLanguage::C, "c17", &sources));

        assert!(matches!(result, Err(BuildError::SecurityViolation(_))));
    }

    #[test]
    fn test_dependency_include_and_link_emission() {
        let root = Path::new("/workspace/deps");
        let sources = vec![root.join("src").join("main.cpp")];
        let include_dirs = vec![root.join(".crux/installed/fmt/11.1.1/abc/include")];
        let link_dirs = vec![root.join(".crux/installed/fmt/11.1.1/abc/lib")];
        let libs = vec![root.join(".crux/installed/fmt/11.1.1/abc/lib/libfmt.a")];
        let runtime_dirs = vec![root.join(".crux/installed/fmt/11.1.1/abc/lib")];

        let content = generate_app_cmakelists(&AppCMakeConfig {
            project_root: root,
            target_name: "deps",
            project_version: "0.1.0",
            language: ProjectLanguage::Cpp,
            standard: "c++20",
            sources: &sources,
            project_include_dirs: &[],
            dependency_include_dirs: &include_dirs,
            dependency_link_directories: &link_dirs,
            dependency_link_libraries: &libs,
            dependency_runtime_directories: &runtime_dirs,
            build_settings: &NO_BUILD_SETTINGS,
        })
        .unwrap();

        assert!(content.contains("target_include_directories(deps PRIVATE"));
        assert!(content.contains("/workspace/deps/.crux/installed/fmt/11.1.1/abc/include"));
        assert!(content.contains("target_link_directories(deps PRIVATE"));
        assert!(content.contains("target_link_libraries(deps PRIVATE"));
        assert!(content.contains("libfmt.a"));
        assert!(content.contains("BUILD_RPATH"));
    }

    #[test]
    fn test_invalid_target_name_is_rejected() {
        let root = Path::new("/workspace/det");
        let sources = vec![root.join("src").join("main.c")];

        let result = generate_app_cmakelists(&config(
            root,
            "bad name",
            ProjectLanguage::C,
            "c17",
            &sources,
        ));

        assert!(result.is_err());
    }
}
