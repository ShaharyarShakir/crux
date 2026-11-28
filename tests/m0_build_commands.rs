//! M0 characterisation: argument vectors used by the existing CMake pipeline.
//!
//! A local fake `cmake` executable records every argv vector as a separate
//! record. No real compiler, generator, CMake binary, or network is involved;
//! production code is only given different binary paths via its existing
//! `with_cmake_bin` / configuration overrides.

#![cfg(unix)]

use crux::build::{BuildConfiguration, BuildEngine, BuildProfile, BuildRequest, CMakeBuildEngine};
use crux::install::{CMakeInstallEngine, InstallEngine, InstallRequest};
use crux::lock::Checksum;
use crux::package::{PackageId, PackageName};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "crux_m0_commands_{prefix}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct Pipeline {
    _root: TempDirGuard,
    source_dir: PathBuf,
    builds_root: PathBuf,
    install_root: PathBuf,
    staging_root: PathBuf,
    fake_cmake: PathBuf,
    c_compiler: PathBuf,
    cxx_compiler: PathBuf,
    command_log: PathBuf,
    config: BuildConfiguration,
}

#[derive(Debug)]
struct CommandRecord {
    env_marker: String,
    destdir: String,
    args: Vec<String>,
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

/// Fake CMake: appends one record per invocation, then emulates just enough of
/// configure/build/install for the engines to complete successfully.
fn fake_cmake_body(log: &Path, absolute_dest: bool) -> String {
    format!(
        r###"#!/bin/sh
set -eu
log={log}
absolute={absolute}

printf 'ENV=%s\n' "${{CRUX_M0_ENV_MARKER-}}" >> "$log"
printf 'DESTD=%s\n' "${{DESTDIR-}}" >> "$log"
printf '%s\n' "$@" >> "$log"
printf '__CRUX_M0_END__\n' >> "$log"

case "${{1-}}" in
  --version)
    printf '%s\n' 'cmake version 3.31.0'
    ;;
  --install)
    build_dir=${{2-}}
    prefix=''
    shift 2
    while [ $# -gt 0 ]; do
      case "$1" in
        --prefix)
          prefix=$2
          shift 2
          ;;
        --config)
          shift 2
          ;;
        *)
          shift
          ;;
      esac
    done
    if [ -z "$prefix" ]; then
      printf 'missing --prefix\n' >&2
      exit 2
    fi
    if [ "$prefix" = '/' ]; then
      destdir=${{DESTDIR-}}
      if [ -z "$destdir" ]; then
        printf 'missing DESTDIR\n' >&2
        exit 2
      fi
      mkdir -p "$destdir/usr/local/include"
      printf 'char header\n' > "$destdir/usr/local/include/char.h"
    else
      mkdir -p "$prefix/include"
      printf 'char header\n' > "$prefix/include/char.h"
    fi
    ;;
  --build)
    build_dir=${{2-}}
    mkdir -p "$build_dir"
    printf 'fake archive\n' > "$build_dir/libchar_lib.a"
    ;;
  -S)
    build_dir=''
    while [ $# -gt 0 ]; do
      case "$1" in
        -B)
          build_dir=${{2-}}
          break
          ;;
        *)
          shift
          ;;
      esac
    done
    if [ -z "$build_dir" ]; then
      printf 'missing -B\n' >&2
      exit 2
    fi
    mkdir -p "$build_dir"
    : > "$build_dir/CMakeCache.txt"
    if [ "$absolute" = '1' ]; then
      printf '# fake install script\nCMAKE_ABSOLUTE_DESTINATION_FILES="/usr/local/include/char.h"\n' > "$build_dir/cmake_install.cmake"
    else
      printf '# fake install script\n' > "$build_dir/cmake_install.cmake"
    fi
    ;;
  *)
    printf 'unexpected fake cmake invocation: %s\n' "$1" >&2
    exit 2
    ;;
esac
exit 0
"###,
        log = sh_quote(&log.display().to_string()),
        absolute = if absolute_dest { "1" } else { "0" },
    )
}

fn setup(absolute_dest: bool) -> Pipeline {
    let root = TempDirGuard::new(if absolute_dest { "abs" } else { "normal" });
    let source_dir = root.path().join("src");
    fs::create_dir_all(&source_dir).unwrap();
    fs::write(source_dir.join("CMakeLists.txt"), "# fake CMake project\n").unwrap();

    let command_log = root.path().join("commands.log");
    let fake_cmake = root.path().join("fake-cmake");
    write_executable(&fake_cmake, &fake_cmake_body(&command_log, absolute_dest));

    let c_compiler = root.path().join("fake-cc");
    let cxx_compiler = root.path().join("fake-cxx");
    write_executable(
        &c_compiler,
        "#!/bin/sh\nprintf '%s\\n' 'gcc (GCC) 14.2.0'\n",
    );
    write_executable(
        &cxx_compiler,
        "#!/bin/sh\nprintf '%s\\n' 'g++ (GCC) 14.2.0'\n",
    );

    let mut extra_env = BTreeMap::new();
    extra_env.insert("CRUX_M0_ENV_MARKER".to_string(), "present".to_string());

    let config = BuildConfiguration {
        profile: BuildProfile::Debug,
        generator: Some("Unix Makefiles".to_string()),
        cmake_args: vec![
            "-DCRUX_M0_FLAG=ON".to_string(),
            "-DCRUX_M0_VALUE=seven".to_string(),
        ],
        c_compiler: Some(c_compiler.clone()),
        cxx_compiler: Some(cxx_compiler.clone()),
        jobs: Some(4),
        build_shared_libs: Some(true),
        extra_env,
    };

    Pipeline {
        _root: root,
        source_dir,
        builds_root: command_log.parent().unwrap().join("builds"),
        install_root: command_log.parent().unwrap().join("installed"),
        staging_root: command_log.parent().unwrap().join("staging"),
        fake_cmake,
        c_compiler,
        cxx_compiler,
        command_log,
        config,
    }
}

fn read_records(log: &Path) -> Vec<CommandRecord> {
    let text = fs::read_to_string(log).unwrap();
    let mut records = Vec::new();
    let mut env_marker = None;
    let mut destdir = None;
    let mut args = Vec::new();

    for line in text.lines() {
        if line == "__CRUX_M0_END__" {
            records.push(CommandRecord {
                env_marker: env_marker.take().unwrap_or_default(),
                destdir: destdir.take().unwrap_or_default(),
                args: std::mem::take(&mut args),
            });
        } else if let Some(value) = line.strip_prefix("ENV=") {
            env_marker = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("DESTD=") {
            destdir = Some(value.to_string());
        } else {
            args.push(line.to_string());
        }
    }

    records
}

fn package_id() -> PackageId {
    PackageId::new(
        PackageName::new("char-lib").unwrap(),
        semver::Version::parse("1.2.3").unwrap(),
    )
}

fn checksum() -> Checksum {
    Checksum::new_sha256("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        .unwrap()
}

fn build(pipeline: &Pipeline) -> crux::build::BuildResult {
    let request = BuildRequest::new(package_id(), pipeline.source_dir.clone(), checksum())
        .with_configuration(pipeline.config.clone());
    CMakeBuildEngine::new(&pipeline.builds_root)
        .with_cmake_bin(&pipeline.fake_cmake)
        .build(&request)
        .unwrap()
}

#[test]
fn configure_and_build_arguments_are_recorded_as_vectors() {
    let pipeline = setup(false);
    let result = build(&pipeline);
    let records = read_records(&pipeline.command_log);

    assert_eq!(records.len(), 3, "expected version probe, configure, build");
    assert_eq!(records[0].args, vec!["--version"]);

    assert_eq!(
        records[1].args,
        vec![
            "-S".to_string(),
            pipeline.source_dir.display().to_string(),
            "-B".to_string(),
            result.build_dir.display().to_string(),
            "-DCMAKE_BUILD_TYPE=Debug".to_string(),
            "-G".to_string(),
            "Unix Makefiles".to_string(),
            "-DBUILD_SHARED_LIBS=ON".to_string(),
            format!("-DCMAKE_C_COMPILER={}", pipeline.c_compiler.display()),
            format!("-DCMAKE_CXX_COMPILER={}", pipeline.cxx_compiler.display()),
            "-DCRUX_M0_FLAG=ON".to_string(),
            "-DCRUX_M0_VALUE=seven".to_string(),
        ],
        "configure argv"
    );
    assert_eq!(
        records[1].env_marker, "present",
        "configure receives extra_env"
    );

    assert_eq!(
        records[2].args,
        vec![
            "--build".to_string(),
            result.build_dir.display().to_string(),
            "--config".to_string(),
            "Debug".to_string(),
            "--parallel".to_string(),
            "4".to_string(),
        ],
        "build argv"
    );
    assert_eq!(
        records[2].env_marker, "",
        "build does not inherit extra_env"
    );

    assert_eq!(
        result.build_dir,
        pipeline
            .builds_root
            .join("char-lib")
            .join("1.2.3")
            .join(result.fingerprint.hex_digest())
            .join("build")
    );
}

#[test]
fn install_arguments_and_standard_staging_are_recorded() {
    let pipeline = setup(false);
    let result = build(&pipeline);
    let install_engine = CMakeInstallEngine::new(&pipeline.install_root, &pipeline.staging_root)
        .with_cmake_bin(&pipeline.fake_cmake);
    let installed = install_engine
        .install(&InstallRequest::new(result.clone()))
        .unwrap();

    let records = read_records(&pipeline.command_log);
    assert_eq!(records.len(), 4, "version probe, configure, build, install");
    let install = &records[3];

    assert_eq!(install.args.len(), 6, "install argv: {install:?}");
    assert_eq!(install.args[0], "--install");
    assert_eq!(install.args[1], result.build_dir.display().to_string());
    assert_eq!(install.args[2], "--prefix");
    assert_eq!(install.args[4], "--config");
    assert_eq!(install.args[5], "Debug");

    let prefix = PathBuf::from(&install.args[3]);
    assert_eq!(prefix.parent(), Some(pipeline.staging_root.as_path()));
    assert_ne!(prefix, pipeline.staging_root);
    assert_eq!(
        install.destdir, "",
        "normal staging uses --prefix, not DESTDIR"
    );

    assert!(
        fs::read_dir(&pipeline.staging_root)
            .unwrap()
            .next()
            .is_none(),
        "staging directory is published by rename, not left behind"
    );
    assert!(installed.install_dir.join("include/char.h").is_file());
    assert_eq!(
        installed.install_dir,
        install_engine.destination_dir(&result.package_id, &result.fingerprint)
    );
}

#[test]
fn install_arguments_and_destdir_staging_are_recorded_for_absolute_destinations() {
    let pipeline = setup(true);
    let result = build(&pipeline);
    let install_engine = CMakeInstallEngine::new(&pipeline.install_root, &pipeline.staging_root)
        .with_cmake_bin(&pipeline.fake_cmake);
    let installed = install_engine
        .install(&InstallRequest::new(result))
        .unwrap();

    let records = read_records(&pipeline.command_log);
    let install = records.last().unwrap();

    assert_eq!(install.args.len(), 6, "install argv: {install:?}");
    assert_eq!(install.args[0], "--install");
    assert_eq!(install.args[2], "--prefix");
    assert_eq!(
        install.args[3], "/",
        "absolute destinations use root prefix"
    );
    assert_eq!(install.args[4], "--config");
    assert_eq!(install.args[5], "Debug");

    let destdir = PathBuf::from(&install.destdir);
    assert_eq!(destdir.parent(), Some(pipeline.staging_root.as_path()));
    assert_ne!(destdir, pipeline.staging_root);

    assert!(installed.install_dir.join("include/char.h").is_file());
    assert!(
        !installed.install_dir.join("usr").exists(),
        "usr/local prefix contents are promoted to the install root"
    );
}
