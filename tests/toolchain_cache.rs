//! Compiler selection, ambient-flag fingerprinting, and package build caching.
//!
//! Most tests use fake compilers and a fake CMake behind
//! `CMakeBuildEngine::with_environment`, so they do not depend on the host
//! environment or on Clang being installed. The `real_` tests use the real
//! CMake and GCC to confirm CMake behaves as the fakes assume.

#![cfg(unix)]

use crux::build::{
    BuildConfiguration, BuildEngine, BuildError, BuildFingerprint, BuildMetadata, BuildRequest,
    BuildResult, CMakeBuildEngine, SelectedCompiler,
};
use crux::install::{CMakeInstallEngine, InstallEngine, InstallRequest};
use crux::lock::Checksum;
use crux::package::{PackageId, PackageName};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn write_executable(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    // A concurrent test's fork can briefly hold the new file open for writing
    // (ETXTBSY). One successful execution shows that has passed.
    while let Err(error) = std::process::Command::new(path).arg("--version").output() {
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::ExecutableFileBusy,
            "{error}"
        );
        std::thread::yield_now();
    }
}

fn write_compiler(path: &Path, version: &str) {
    write_executable(path, &format!("#!/bin/sh\nprintf '%s\\n' '{version}'\n"));
}

/// Records argv and flag variables, caches the compilers like CMake does, and
/// obeys the marker files `header-only` and `fail-build`.
const FAKE_CMAKE: &str = r##"#!/bin/sh
PATH='@PATH@'
root='@ROOT@'
case "$1" in
  --version) echo 'cmake version 3.31.0' ;;
  -S)
    printf '%s\n' "$@" > "$root/configure.args"
    printf 'CFLAGS=%s\nCXXFLAGS=%s\nLDFLAGS=%s\n' \
      "${CFLAGS-<unset>}" "${CXXFLAGS-<unset>}" "${LDFLAGS-<unset>}" > "$root/configure.env"
    mkdir -p "$4"
    : > "$4/CMakeCache.txt"
    for arg in "$@"; do
      case "$arg" in
        -DCMAKE_C_COMPILER=*) echo "CMAKE_C_COMPILER:FILEPATH=${arg#*=}" >> "$4/CMakeCache.txt" ;;
        -DCMAKE_CXX_COMPILER=*) echo "CMAKE_CXX_COMPILER:FILEPATH=${arg#*=}" >> "$4/CMakeCache.txt" ;;
      esac
    done
    echo '# install script' > "$4/cmake_install.cmake" ;;
  --build)
    echo build >> "$root/builds.log"
    if [ -e "$root/fail-build" ]; then exit 1; fi
    if [ ! -e "$root/header-only" ]; then echo archive > "$2/libx.a"; fi ;;
  --install)
    mkdir -p "$4/include"
    cp "$root/header.txt" "$4/include/x.h" ;;
esac
"##;

struct Fixture {
    root: PathBuf,
    env: BTreeMap<String, String>,
    config: BuildConfiguration,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "crux_toolchain_{name}_{}_{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = temp_root(name);
        let bin = root.join("bin");
        write_compiler(&bin.join("gcc"), "gcc (GCC) 14.2.0");
        write_compiler(&bin.join("g++"), "g++ (GCC) 14.2.0");
        write_compiler(&bin.join("clang"), "clang version 19.1.0");
        write_compiler(&bin.join("clang++"), "clang version 19.1.0");
        let script = FAKE_CMAKE
            .replace("@PATH@", &std::env::var("PATH").unwrap())
            .replace("@ROOT@", root.to_str().unwrap());
        write_executable(&root.join("cmake"), &script);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/CMakeLists.txt"), "# fake\n").unwrap();
        fs::write(root.join("header.txt"), "first\n").unwrap();

        let env = BTreeMap::from([("PATH".to_string(), bin.display().to_string())]);
        let config = BuildConfiguration {
            generator: Some("Unix Makefiles".to_string()),
            ..Default::default()
        };
        Self { root, env, config }
    }

    fn set(&mut self, key: &str, value: &str) -> &mut Self {
        self.env.insert(key.to_string(), value.to_string());
        self
    }

    fn bin(&self, name: &str) -> PathBuf {
        self.root.join("bin").join(name)
    }

    fn engine(&self) -> CMakeBuildEngine {
        CMakeBuildEngine::new(self.root.join("builds"))
            .with_cmake_bin(self.root.join("cmake"))
            .with_environment(self.env.clone())
    }

    fn request(&self) -> BuildRequest {
        BuildRequest::new(package_id(), self.root.join("src"), checksum())
            .with_configuration(self.config.clone())
    }

    fn build(&self) -> BuildResult {
        self.engine().build(&self.request()).unwrap()
    }

    fn fingerprint(&self) -> BuildFingerprint {
        self.build().fingerprint
    }

    fn configure_args(&self) -> Vec<String> {
        let text = fs::read_to_string(self.root.join("configure.args")).unwrap();
        text.lines().map(str::to_string).collect()
    }

    fn build_count(&self) -> usize {
        fs::read_to_string(self.root.join("builds.log"))
            .map(|log| log.lines().count())
            .unwrap_or(0)
    }

    /// The compilers recorded in metadata, checked against CMake's arguments.
    fn selected(&self, result: &BuildResult) -> (SelectedCompiler, SelectedCompiler) {
        let selected = metadata(result).selected_toolchain.unwrap();
        let (c, cxx) = (selected.c_compiler.unwrap(), selected.cxx_compiler.unwrap());
        let args = self.configure_args();
        for (var, compiler) in [("C", &c), ("CXX", &cxx)] {
            let mut expected = format!("-DCMAKE_{var}_COMPILER={}", compiler.path.display());
            for arg in &compiler.args {
                expected = format!("{expected};{arg}");
            }
            assert!(args.contains(&expected), "{expected} not in {args:?}");
        }
        (c, cxx)
    }
}

fn package_id() -> PackageId {
    PackageId::new(
        PackageName::new("x").unwrap(),
        semver::Version::new(1, 0, 0),
    )
}

fn checksum() -> Checksum {
    Checksum::new_sha256("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        .unwrap()
}

fn metadata(result: &BuildResult) -> BuildMetadata {
    toml::from_str(&fs::read_to_string(&result.metadata_path).unwrap()).unwrap()
}

fn rewrite_metadata(result: &BuildResult, change: impl FnOnce(&mut BuildMetadata)) {
    let mut value = metadata(result);
    change(&mut value);
    fs::write(&result.metadata_path, toml::to_string(&value).unwrap()).unwrap();
}

// ---- Compiler resolution ----

#[test]
fn default_gcc_is_selected_recorded_and_passed_to_cmake() {
    let fixture = Fixture::new("default");
    let result = fixture.build();
    let (c, cxx) = fixture.selected(&result);
    assert_eq!(c.path, fixture.bin("gcc"));
    assert_eq!(cxx.path, fixture.bin("g++"));
    let toolchain = metadata(&result).toolchain;
    assert_eq!(toolchain.c_compiler_version.unwrap(), "gcc (GCC) 14.2.0");
}

#[test]
fn cc_and_cxx_are_resolved_independently() {
    for (cc, cxx) in [(true, true), (true, false), (false, true)] {
        let mut fixture = Fixture::new("env");
        let baseline = fixture.fingerprint();
        if cc {
            fixture.set("CC", "clang");
        }
        if cxx {
            fixture.set("CXX", "clang++");
        }
        let result = fixture.build();
        assert_ne!(result.fingerprint, baseline, "CC={cc} CXX={cxx}");
        let (c, cpp) = fixture.selected(&result);
        assert_eq!(c.path, fixture.bin(if cc { "clang" } else { "gcc" }));
        assert_eq!(cpp.path, fixture.bin(if cxx { "clang++" } else { "g++" }));
        let versions = metadata(&result).toolchain;
        assert_eq!(
            versions.c_compiler_version.unwrap().starts_with("clang"),
            cc
        );
        assert_eq!(
            versions.cxx_compiler_version.unwrap().starts_with("clang"),
            cxx
        );
    }
}

#[test]
fn a_compiler_with_the_same_version_but_another_path_is_distinguished() {
    let mut fixture = Fixture::new("same_version");
    let baseline = fixture.fingerprint();
    let other = fixture.root.join("other/gcc");
    write_compiler(&other, "gcc (GCC) 14.2.0");
    fixture.set("CC", other.to_str().unwrap());
    assert_ne!(fixture.fingerprint(), baseline);
}

#[test]
fn naming_the_default_compiler_keeps_the_default_fingerprint() {
    let mut fixture = Fixture::new("explicit_default");
    let baseline = fixture.fingerprint();
    let gcc = fixture.bin("gcc");
    fixture.set("CC", gcc.to_str().unwrap()).set("CXX", "g++");
    assert_eq!(fixture.fingerprint(), baseline);
}

#[test]
fn explicit_overrides_take_precedence_over_the_environment() {
    let mut fixture = Fixture::new("explicit");
    fixture.set("CC", "clang").set("CXX", "does-not-exist");
    fixture.config.c_compiler = Some(fixture.bin("gcc"));
    fixture.config.cxx_compiler = Some(fixture.bin("g++"));
    let result = fixture.build();
    let (c, cxx) = fixture.selected(&result);
    assert_eq!((c.path, cxx.path), (fixture.bin("gcc"), fixture.bin("g++")));

    fixture.config.c_compiler = None;
    fixture.config.cxx_compiler = None;
    let engine = fixture
        .engine()
        .with_c_compiler(fixture.bin("gcc"))
        .with_cxx_compiler(fixture.bin("g++"));
    let result = engine.build(&fixture.request().with_force(true)).unwrap();
    let (c, cxx) = fixture.selected(&result);
    assert_eq!((c.path, cxx.path), (fixture.bin("gcc"), fixture.bin("g++")));
}

#[test]
fn invalid_overrides_are_reported_and_nothing_is_configured() {
    for source in ["config", "engine", "CC", "CXX"] {
        let mut fixture = Fixture::new("invalid");
        let mut engine_override = None;
        match source {
            "config" => fixture.config.c_compiler = Some("/no/such/compiler".into()),
            "engine" => engine_override = Some("/no/such/compiler"),
            key => {
                fixture.set(key, "/no/such/compiler");
            }
        }
        let mut engine = fixture.engine();
        if let Some(path) = engine_override {
            engine = engine.with_c_compiler(path);
        }
        let error = engine.build(&fixture.request()).unwrap_err();
        assert!(
            matches!(error, BuildError::ToolchainDetectionFailed(_)),
            "{source}: {error}"
        );
        assert!(error.to_string().contains("/no/such/compiler"), "{error}");
        assert!(!fixture.root.join("configure.args").exists(), "{source}");
        assert!(!engine.is_fresh(&fixture.request()));
    }
}

#[test]
fn compiler_paths_with_spaces_and_compiler_arguments_are_kept_apart() {
    let mut fixture = Fixture::new("spaces");
    let spaced = fixture.root.join("dir with spaces/my cc");
    write_compiler(&spaced, "gcc (GCC) 14.2.0");
    fixture
        .set("CC", spaced.to_str().unwrap())
        .set("CXX", "clang++ -stdlib=libc++");
    let result = fixture.build();
    let (c, cxx) = fixture.selected(&result);
    assert_eq!(c.path, spaced);
    assert!(c.args.is_empty());
    assert_eq!(cxx.path, fixture.bin("clang++"));
    assert_eq!(cxx.args, ["-stdlib=libc++"]);

    let with_argument = result.fingerprint;
    fixture.set("CXX", "clang++");
    assert_ne!(fixture.fingerprint(), with_argument);
}

#[test]
fn a_missing_gcc_falls_back_and_the_fallback_is_fingerprinted() {
    let fixture = Fixture::new("fallback");
    fs::remove_file(fixture.bin("gcc")).unwrap();
    fs::remove_file(fixture.bin("g++")).unwrap();
    let result = fixture.build();
    let (c, cxx) = fixture.selected(&result);
    assert_eq!(c.path, fixture.bin("clang"));
    assert_eq!(cxx.path, fixture.bin("clang++"));
}

#[test]
fn a_compiler_definition_in_cmake_args_is_what_gets_fingerprinted() {
    let mut fixture = Fixture::new("cmake_args");
    fixture
        .config
        .cmake_args
        .push("-DCMAKE_C_COMPILER=clang".to_string());
    let result = fixture.build();
    let record = metadata(&result);
    assert_eq!(
        record.toolchain.c_compiler_version.unwrap(),
        "clang version 19.1.0"
    );
    assert_eq!(
        record.selected_toolchain.unwrap().c_compiler.unwrap().path,
        fixture.bin("clang")
    );
    let passed: Vec<_> = fixture
        .configure_args()
        .into_iter()
        .filter(|a| a.starts_with("-DCMAKE_C_COMPILER="))
        .collect();
    assert_eq!(passed, ["-DCMAKE_C_COMPILER=clang"]);
}

// ---- Fingerprints ----

#[test]
fn a_clean_default_environment_keeps_the_previous_fingerprint() {
    let fixture = Fixture::new("compatible");
    let toolchain = fixture.engine().detect_toolchain(&fixture.config).unwrap();
    // `compute` is the unchanged algorithm pinned by the M0 golden digest.
    let previous =
        BuildFingerprint::compute(&package_id(), &checksum(), &toolchain, &fixture.config);
    assert_eq!(fixture.fingerprint(), previous);
    let env = fs::read_to_string(fixture.root.join("configure.env")).unwrap();
    assert_eq!(env, "CFLAGS=<unset>\nCXXFLAGS=<unset>\nLDFLAGS=<unset>\n");
}

#[test]
fn ambient_flags_are_fingerprinted_and_passed_to_cmake() {
    let mut seen = std::collections::BTreeSet::new();
    let mut fixture = Fixture::new("flags");
    assert!(seen.insert(fixture.fingerprint()));
    for key in ["CFLAGS", "CXXFLAGS", "LDFLAGS"] {
        // Unset, empty, a value, and the same flags reordered all differ.
        for value in ["", "-O1 -g", "-g -O1"] {
            fixture.env.retain(|k, _| k == "PATH");
            fixture.set(key, value);
            let result = fixture.build();
            assert!(seen.insert(result.fingerprint.clone()), "{key}='{value}'");
            assert_eq!(fixture.fingerprint(), result.fingerprint, "deterministic");

            let env = fs::read_to_string(fixture.root.join("configure.env")).unwrap();
            assert!(env.contains(&format!("{key}={value}\n")), "{env}");
            let flags = metadata(&result).selected_toolchain.unwrap().ambient_flags;
            assert_eq!(
                flags,
                BTreeMap::from([(key.to_string(), value.to_string())])
            );
        }
    }
}

#[test]
fn unrelated_variables_do_not_change_the_fingerprint() {
    let mut fixture = Fixture::new("unrelated");
    let baseline = fixture.fingerprint();
    fixture.set("CPPFLAGS", "-DX").set("HOME", "/elsewhere");
    assert_eq!(fixture.fingerprint(), baseline);
}

#[test]
fn extra_env_overrides_the_ambient_value() {
    let mut fixture = Fixture::new("extra_env");
    fixture
        .config
        .extra_env
        .insert("CFLAGS".to_string(), "-DFROM_CONFIG".to_string());
    let baseline = fixture.fingerprint();
    fixture.set("CFLAGS", "-DAMBIENT");
    assert_eq!(fixture.fingerprint(), baseline);
    let env = fs::read_to_string(fixture.root.join("configure.env")).unwrap();
    assert!(env.contains("CFLAGS=-DFROM_CONFIG\n"), "{env}");
}

// ---- Poisoned and legacy cache entries ----

/// Makes `result` look like an entry written by an earlier Crux whose CMake
/// tree contains `cache`.
fn make_legacy(result: &BuildResult, cache: &str) {
    rewrite_metadata(result, |m| m.selected_toolchain = None);
    fs::write(result.build_dir.join("CMakeCache.txt"), cache).unwrap();
}

#[test]
fn poisoned_legacy_entries_are_rebuilt_and_reinstalled() {
    for cache in ["compiler", "flags"] {
        let fixture = Fixture::new("poisoned");
        let install =
            CMakeInstallEngine::new(fixture.root.join("installed"), fixture.root.join("staging"))
                .with_cmake_bin(fixture.root.join("cmake"));
        let first = fixture.build();
        install
            .install(&InstallRequest::new(first.clone()))
            .unwrap();
        make_legacy(
            &first,
            &match cache {
                "compiler" => format!(
                    "CMAKE_C_COMPILER:FILEPATH={}\n",
                    fixture.bin("clang").display()
                ),
                _ => "CMAKE_C_FLAGS:STRING=-O0\n".to_string(),
            },
        );
        assert!(!fixture.engine().is_fresh(&fixture.request()), "{cache}");

        fs::write(fixture.root.join("header.txt"), "second build\n").unwrap();
        let rebuilt = fixture.build();
        assert!(!rebuilt.reused_cache, "{cache}");
        assert_eq!(rebuilt.fingerprint, first.fingerprint);
        // Even a later cache hit must not keep the old installation.
        let hit = fixture.build();
        assert!(hit.reused_cache);
        let installed = install.install(&InstallRequest::new(hit.clone())).unwrap();
        assert!(!installed.reused_install, "{cache}");
        assert_eq!(
            fs::read_to_string(installed.install_dir.join("include/x.h")).unwrap(),
            "second build\n"
        );
        assert!(
            install
                .install(&InstallRequest::new(hit))
                .unwrap()
                .reused_install
        );
    }
}

#[test]
fn healthy_legacy_entries_are_still_reused() {
    let fixture = Fixture::new("legacy_ok");
    let first = fixture.build();
    // Earlier Crux let CMake find `cc`, an alias of the same GCC.
    std::os::unix::fs::symlink(fixture.bin("gcc"), fixture.bin("cc")).unwrap();
    make_legacy(
        &first,
        &format!(
            "CMAKE_C_COMPILER:FILEPATH={}\nCMAKE_C_FLAGS:STRING=\n",
            fixture.bin("cc").display()
        ),
    );
    assert!(fixture.build().reused_cache);
    assert_eq!(fixture.build_count(), 1);
}

// ---- Header-only and cache validity ----

fn header_only(name: &str) -> Fixture {
    let fixture = Fixture::new(name);
    fs::write(fixture.root.join("header-only"), "").unwrap();
    fixture
}

#[test]
fn a_successful_build_without_artifacts_is_reused() {
    let fixture = header_only("header_only");
    let first = fixture.build();
    assert!(first.artifacts.is_empty());
    assert!(!first.reused_cache);
    assert!(fixture.engine().is_fresh(&fixture.request()));
    let second = fixture.build();
    assert!(second.reused_cache);
    assert!(second.artifacts.is_empty());
    assert_eq!(fixture.build_count(), 1);
}

#[test]
fn invalid_metadata_is_not_reused() {
    type Break = fn(&BuildResult);
    let cases: [(&str, Break); 7] = [
        ("missing", |r| fs::remove_file(&r.metadata_path).unwrap()),
        ("truncated", |r| {
            let text = fs::read_to_string(&r.metadata_path).unwrap();
            fs::write(&r.metadata_path, &text[..text.len() / 2]).unwrap();
        }),
        ("unsuccessful", |r| {
            rewrite_metadata(r, |m| m.success = false)
        }),
        ("format version", |r| {
            rewrite_metadata(r, |m| m.format_version = 999)
        }),
        ("exit code", |r| {
            rewrite_metadata(r, |m| m.build_exit_code = 1)
        }),
        ("no CMake cache", |r| {
            fs::remove_file(r.build_dir.join("CMakeCache.txt")).unwrap()
        }),
        ("no build dir", |r| {
            fs::remove_dir_all(&r.build_dir).unwrap()
        }),
    ];
    for (name, break_entry) in cases {
        let fixture = header_only("invalid_metadata");
        break_entry(&fixture.build());
        assert!(!fixture.build().reused_cache, "{name}");
    }
}

#[test]
fn a_failed_build_leaves_nothing_reusable() {
    let fixture = header_only("failed");
    fs::write(fixture.root.join("fail-build"), "").unwrap();
    for _ in 0..2 {
        assert!(fixture.engine().build(&fixture.request()).is_err());
    }
    assert_eq!(fixture.build_count(), 2);
    fs::remove_file(fixture.root.join("fail-build")).unwrap();
    let good = fixture.build();
    assert!(!good.reused_cache);

    // A failed forced rebuild must also discard the earlier success.
    fs::write(fixture.root.join("fail-build"), "").unwrap();
    let forced = fixture.request().with_force(true);
    assert!(fixture.engine().build(&forced).is_err());
    assert!(!good.metadata_path.exists());
    assert!(!fixture.engine().is_fresh(&fixture.request()));
    assert!(fixture.engine().build(&fixture.request()).is_err());
}

#[test]
fn missing_or_resized_artifacts_invalidate_the_cache() {
    let fixture = Fixture::new("artifacts");
    let first = fixture.build();
    assert_eq!(first.artifacts.len(), 1);
    assert!(fixture.build().reused_cache);

    fs::write(&first.artifacts[0].path, "a different size").unwrap();
    assert!(!fixture.build().reused_cache);
    fs::remove_file(&first.artifacts[0].path).unwrap();
    assert!(!fixture.build().reused_cache);
    assert!(fixture.build().reused_cache);
}

// ---- Real CMake ----

fn real_request(root: &Path, cmakelists: &str) -> BuildRequest {
    let source = root.join("src");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("CMakeLists.txt"), cmakelists).unwrap();
    fs::write(source.join("x.h"), "#define X 1\n").unwrap();
    fs::write(source.join("x.c"), "int x(void) { return 1; }\n").unwrap();
    BuildRequest::new(package_id(), source, checksum())
}

fn host_path() -> String {
    std::env::var("PATH").unwrap()
}

const STATIC_LIBRARY: &str =
    "cmake_minimum_required(VERSION 3.15)\nproject(x C)\nadd_library(x STATIC x.c)\n";

#[test]
fn real_header_only_package_is_built_once() {
    let root = temp_root("real_header_only");
    let request = real_request(
        &root,
        "cmake_minimum_required(VERSION 3.15)\nproject(x C)\n\
         add_library(x INTERFACE)\ninstall(FILES x.h DESTINATION include)\n",
    );
    let engine = CMakeBuildEngine::new(root.join("builds"));
    let first = engine.build(&request).unwrap();
    assert!(first.artifacts.is_empty(), "{:?}", first.artifacts);
    assert!(engine.build(&request).unwrap().reused_cache);

    let install = CMakeInstallEngine::new(root.join("installed"), root.join("staging"));
    let installed = install.install(&InstallRequest::new(first)).unwrap();
    assert!(installed.install_dir.join("include/x.h").is_file());
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn real_cmake_uses_the_selected_compiler_and_arguments() {
    let root = temp_root("real_compiler");
    let wrapper = root.join("dir with spaces/wrapped cc");
    let log = root.join("wrapper.log");
    write_executable(
        &wrapper,
        &format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\nexec gcc \"$@\"\n",
            log.display()
        ),
    );
    let engine = CMakeBuildEngine::new(root.join("builds")).with_environment([
        ("PATH".to_string(), host_path()),
        ("CC".to_string(), wrapper.display().to_string()),
    ]);
    let request = real_request(&root, STATIC_LIBRARY);
    let result = engine.build(&request).unwrap();
    assert!(fs::read_to_string(&log).unwrap().contains("x.c"));
    assert!(engine.build(&request).unwrap().reused_cache);
    let cache = fs::read_to_string(result.build_dir.join("CMakeCache.txt")).unwrap();
    assert!(cache.contains(wrapper.to_str().unwrap()), "{cache}");

    // Compiler plus a fixed argument, as `CC="ccache gcc"` is written.
    let engine = CMakeBuildEngine::new(root.join("builds")).with_environment([
        ("PATH".to_string(), host_path()),
        ("CC".to_string(), "env gcc".to_string()),
    ]);
    let result = engine.build(&request).unwrap();
    assert!(engine.build(&request).unwrap().reused_cache);
    // The same tree is understood without the recorded selection.
    rewrite_metadata(&result, |m| m.selected_toolchain = None);
    assert!(engine.build(&request).unwrap().reused_cache);
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn real_legacy_entry_is_reused_unless_cmake_used_other_flags() {
    let root = temp_root("real_legacy");
    let request = real_request(&root, STATIC_LIBRARY);
    let clean =
        CMakeBuildEngine::new(root.join("builds")).with_environment([("PATH", host_path())]);
    let first = clean.build(&request).unwrap();
    rewrite_metadata(&first, |m| m.selected_toolchain = None);
    assert!(clean.build(&request).unwrap().reused_cache);

    // What an earlier Crux did with CFLAGS set: the default fingerprint
    // directory, configured with flags the fingerprint did not mention.
    let status = std::process::Command::new("cmake")
        .arg("-S")
        .arg(&request.source_dir)
        .arg("-B")
        .arg(&first.build_dir)
        .arg("-DCMAKE_C_FLAGS=-DPOISON")
        .output()
        .unwrap()
        .status;
    assert!(status.success());
    let rebuilt = clean.build(&request).unwrap();
    assert!(!rebuilt.reused_cache);
    let cache = fs::read_to_string(rebuilt.build_dir.join("CMakeCache.txt")).unwrap();
    assert!(!cache.contains("POISON"), "{cache}");
    fs::remove_dir_all(&root).unwrap();
}

#[test]
fn inherited_search_paths_are_fingerprinted_with_order_and_emptiness() {
    let mut fixture = Fixture::new("search_paths");
    let clean = fixture.fingerprint();

    let mut seen = std::collections::BTreeSet::from([clean.clone()]);
    for key in ["CMAKE_PREFIX_PATH", "PKG_CONFIG_PATH"] {
        for value in ["", "/a:/b", "/b:/a"] {
            fixture.set(key, value);
            let digest = fixture.fingerprint();
            assert_eq!(digest, fixture.fingerprint());
            assert!(seen.insert(digest), "{key}={value:?} was not told apart");
        }
        fixture.env.remove(key);
        assert_eq!(fixture.fingerprint(), clean);
    }

    // An `extra_env` value replaces the inherited one, which then has no effect.
    fixture
        .config
        .extra_env
        .insert("CMAKE_PREFIX_PATH".to_string(), "/fixed".to_string());
    let fixed = fixture.fingerprint();
    fixture.set("CMAKE_PREFIX_PATH", "/ignored");
    assert_eq!(fixture.fingerprint(), fixed);
}
