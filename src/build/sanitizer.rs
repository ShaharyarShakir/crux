//! Expansion of `[build]` settings into compiler/linker arguments, and
//! detection of unsupported sanitizer configurations.
//!
//! Everything here returns individual arguments; nothing is ever joined into
//! a shell string. Requested sanitizers are never silently dropped: if the
//! toolchain cannot build and link an instrumented program, the build stops
//! with a [`BuildError::SanitizerUnsupported`] that carries the compiler's own
//! output.

use super::BuildError;
use super::toolchain::{CompilerFamily, CompilerInfo};
use crate::core::process::{describe_command, run_with_timeout};
use crate::project::build_settings::BuildSettings;
use crate::project::manifest::ProjectLanguage;
use std::process::Command;
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

/// Arguments that enable the requested sanitizers; used for both compiling
/// and linking so instrumentation and runtime libraries always match.
///
/// Empty when no sanitizer is requested.
pub fn sanitizer_flags(settings: &BuildSettings) -> Vec<String> {
    let names: Vec<&str> = settings.sanitizers().iter().map(|s| s.name()).collect();
    if names.is_empty() {
        return Vec::new();
    }
    let mut flags = vec![format!("-fsanitize={}", names.join(","))];
    if settings.address_sanitizer {
        // Readable stack traces in ASan reports.
        flags.push("-fno-omit-frame-pointer".to_string());
    }
    flags
}

/// Full compile argument list for the application's language: sanitizer
/// flags first, then the user's flags in declared order, so a user flag can
/// refine a sanitizer option.
pub fn effective_compile_flags(settings: &BuildSettings, language: ProjectLanguage) -> Vec<String> {
    let mut flags = sanitizer_flags(settings);
    match language {
        ProjectLanguage::C => flags.extend(settings.c_flags.iter().cloned()),
        ProjectLanguage::Cpp => flags.extend(settings.cxx_flags.iter().cloned()),
        ProjectLanguage::Other => {}
    }
    flags
}

/// Full link argument list: sanitizer flags first, then the user's linker
/// flags in declared order.
pub fn effective_link_flags(settings: &BuildSettings) -> Vec<String> {
    let mut flags = sanitizer_flags(settings);
    flags.extend(settings.linker_flags.iter().cloned());
    flags
}

/// Verifies that `compiler` can compile and link a program with the
/// requested sanitizers. Does nothing when none are requested.
///
/// Support is established by actually building a trivial program, not by
/// guessing from the compiler name or version. The probe only compiles and
/// links; it does not run the result.
pub fn check_sanitizer_support(
    compiler: &CompilerInfo,
    settings: &BuildSettings,
) -> Result<(), BuildError> {
    let flags = sanitizer_flags(settings);
    if flags.is_empty() {
        return Ok(());
    }
    let requested = settings
        .sanitizers()
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let compiler_label = format!("{} {}", compiler.family, compiler.version);

    if !matches!(
        compiler.family,
        CompilerFamily::Gcc | CompilerFamily::Clang | CompilerFamily::AppleClang
    ) {
        return Err(BuildError::SanitizerUnsupported {
            compiler: compiler_label,
            sanitizers: requested,
            reason: "Crux supports sanitizers only with GCC and Clang (-fsanitize=...)".to_string(),
            command: String::new(),
            output: String::new(),
        });
    }

    let (file_name, source) = match compiler.language {
        ProjectLanguage::C => ("probe.c", "int main(void) { return 0; }\n"),
        ProjectLanguage::Cpp => ("probe.cpp", "int main() { return 0; }\n"),
        ProjectLanguage::Other => {
            return Err(BuildError::InvalidConfiguration(
                "sanitizers require a C or C++ project".to_string(),
            ));
        }
    };

    let dir = std::env::temp_dir().join(format!(
        "crux_sanitizer_probe_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).map_err(BuildError::Io)?;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());

    let source_path = dir.join(file_name);
    std::fs::write(&source_path, source).map_err(BuildError::Io)?;
    let binary_path = dir.join("probe_bin");

    let mut command = Command::new(&compiler.path);
    command
        .args(&flags)
        .arg(&source_path)
        .arg("-o")
        .arg(&binary_path);
    let display = describe_command(&command);

    let output = run_with_timeout(&mut command, PROBE_TIMEOUT).map_err(|err| {
        BuildError::SanitizerUnsupported {
            compiler: compiler_label.clone(),
            sanitizers: requested.clone(),
            reason: format!("the compiler could not be executed: {err}"),
            command: display.clone(),
            output: String::new(),
        }
    })?;

    if output.success() && !output.timed_out {
        return Ok(());
    }
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stdout));
    Err(BuildError::SanitizerUnsupported {
        compiler: compiler_label,
        sanitizers: requested,
        reason: if output.timed_out {
            "the sanitizer probe timed out".to_string()
        } else {
            "the compiler failed to compile and link a trivial program with these flags".to_string()
        },
        command: display,
        output: text.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn settings(address: bool, undefined: bool) -> BuildSettings {
        BuildSettings {
            address_sanitizer: address,
            undefined_sanitizer: undefined,
            ..Default::default()
        }
    }

    #[test]
    fn no_sanitizers_means_no_flags() {
        assert!(sanitizer_flags(&BuildSettings::default()).is_empty());
    }

    #[test]
    fn address_adds_frame_pointer_flag() {
        assert_eq!(
            sanitizer_flags(&settings(true, false)),
            ["-fsanitize=address", "-fno-omit-frame-pointer"]
        );
    }

    #[test]
    fn undefined_alone_and_combined() {
        assert_eq!(
            sanitizer_flags(&settings(false, true)),
            ["-fsanitize=undefined"]
        );
        assert_eq!(
            sanitizer_flags(&settings(true, true)),
            ["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]
        );
    }

    #[test]
    fn compile_flags_select_language_and_preserve_order() {
        let s = BuildSettings {
            c_flags: vec!["-Wall".into(), "-Wextra".into()],
            cxx_flags: vec!["-Wpedantic".into()],
            linker_flags: vec!["-Wl,--as-needed".into()],
            undefined_sanitizer: true,
            ..Default::default()
        };
        assert_eq!(
            effective_compile_flags(&s, ProjectLanguage::C),
            ["-fsanitize=undefined", "-Wall", "-Wextra"]
        );
        assert_eq!(
            effective_compile_flags(&s, ProjectLanguage::Cpp),
            ["-fsanitize=undefined", "-Wpedantic"]
        );
        assert_eq!(
            effective_link_flags(&s),
            ["-fsanitize=undefined", "-Wl,--as-needed"]
        );
    }

    fn fake_compiler(dir: &std::path::Path, family: CompilerFamily, script: &str) -> CompilerInfo {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-cc");
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        CompilerInfo {
            family,
            language: ProjectLanguage::C,
            version: "1.0".to_string(),
            target_triple: "test".to_string(),
            path,
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "crux_sanitizer_test_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn probe_is_skipped_without_sanitizers() {
        let dir = temp_dir("skip");
        let cc = fake_compiler(&dir, CompilerFamily::Gcc, "exit 99");
        assert!(check_sanitizer_support(&cc, &BuildSettings::default()).is_ok());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn supported_toolchain_passes_and_receives_individual_args() {
        let dir = temp_dir("ok");
        let log = dir.join("args.log");
        let cc = fake_compiler(
            &dir,
            CompilerFamily::Clang,
            &format!("for a in \"$@\"; do echo \"$a\" >> {}; done", log.display()),
        );
        check_sanitizer_support(&cc, &settings(true, true)).unwrap();
        let args = std::fs::read_to_string(&log).unwrap();
        assert!(args.lines().any(|l| l == "-fsanitize=address,undefined"));
        assert!(args.lines().any(|l| l == "-fno-omit-frame-pointer"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn failing_probe_reports_compiler_output() {
        let dir = temp_dir("fail");
        let cc = fake_compiler(
            &dir,
            CompilerFamily::Gcc,
            "echo 'cannot find libasan' >&2; exit 1",
        );
        let err = check_sanitizer_support(&cc, &settings(true, false)).unwrap_err();
        match err {
            BuildError::SanitizerUnsupported {
                sanitizers,
                output,
                command,
                ..
            } => {
                assert_eq!(sanitizers, "AddressSanitizer");
                assert_eq!(output, "cannot find libasan");
                assert!(command.contains("-fsanitize=address"), "{command}");
            }
            other => panic!("unexpected error: {other}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn msvc_is_rejected_without_running_anything() {
        let dir = temp_dir("msvc");
        let cc = fake_compiler(&dir, CompilerFamily::Msvc, "exit 0");
        let err = check_sanitizer_support(&cc, &settings(false, true)).unwrap_err();
        assert!(matches!(err, BuildError::SanitizerUnsupported { .. }));
        assert!(err.to_string().contains("GCC and Clang"), "{err}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
