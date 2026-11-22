//! Shell-level regression tests for `install/install.sh` checksum verification.
//!
//! Each test builds a fake release (a tiny `crux` shell script packaged like a
//! real release archive) in a temporary directory and serves it through a
//! `file://` mirror via `CRUX_RELEASE_BASE_URL`. `HOME` and the install
//! directory are redirected into the sandbox. No network access is performed.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static COUNTER: AtomicUsize = AtomicUsize::new(1);
const VERSION: &str = "v0.2.1";
const ARCHIVE: &str = "crux-v0.2.1-x86_64-unknown-linux-gnu.tar.gz";

struct Sandbox {
    root: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

impl Sandbox {
    fn release_dir(&self) -> PathBuf {
        self.root.join("release").join(VERSION)
    }
    fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }
    fn installed(&self) -> bool {
        self.bin_dir().join("crux").is_file()
    }
}

fn tool_available(tool: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {tool}"))
        .output()
        .is_ok_and(|out| out.status.success())
}

/// The installer only supports Linux x86_64 and needs curl, sha256sum, tar.
fn supported() -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64"))
        && ["bash", "curl", "sha256sum", "tar"]
            .iter()
            .all(|tool| tool_available(tool))
}

/// Creates a sandbox containing a valid release archive; returns it with the
/// archive's lowercase SHA-256.
fn sandbox(prefix: &str) -> (Sandbox, String) {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!(
        "crux_install_sh_{prefix}_{}_{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    let pkg = root
        .join("pkg")
        .join(format!("crux-{VERSION}-x86_64-unknown-linux-gnu"));
    fs::create_dir_all(&pkg).unwrap();
    let bin = pkg.join("crux");
    fs::write(&bin, "#!/bin/sh\necho \"crux 0.2.1\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let sandbox = Sandbox { root };
    fs::create_dir_all(sandbox.release_dir()).unwrap();
    let status = Command::new("tar")
        .arg("czf")
        .arg(sandbox.release_dir().join(ARCHIVE))
        .arg("-C")
        .arg(sandbox.root.join("pkg"))
        .arg(format!("crux-{VERSION}-x86_64-unknown-linux-gnu"))
        .status()
        .unwrap();
    assert!(status.success());
    let digest = format!(
        "{:x}",
        Sha256::digest(fs::read(sandbox.release_dir().join(ARCHIVE)).unwrap())
    );
    (sandbox, digest)
}

fn run_installer(sandbox: &Sandbox, sums: &str, args: &[&str], base_url: Option<&str>) -> Output {
    fs::write(sandbox.release_dir().join("SHA256SUMS"), sums).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("install/install.sh");
    let base = base_url
        .map(str::to_string)
        .unwrap_or_else(|| format!("file://{}", sandbox.release_dir().display()));
    Command::new("bash")
        .arg(script)
        .args(args)
        .env("CRUX_RELEASE_BASE_URL", base)
        .env("CRUX_INSTALL_DIR", sandbox.bin_dir())
        .env("HOME", &sandbox.root)
        .env_remove("CRUX_VERSION")
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_rejected(sandbox: &Sandbox, output: &Output, expected: &str) {
    assert!(
        !output.status.success(),
        "installer must fail: {}",
        stderr(output)
    );
    assert!(
        stderr(output).contains(expected),
        "expected '{expected}' in stderr:\n{}",
        stderr(output)
    );
    assert!(
        !sandbox.installed(),
        "nothing may be installed after a failed verification"
    );
    let leftovers: Vec<_> = fs::read_dir(sandbox.bin_dir())
        .map(|entries| entries.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "temporary files left in bin dir: {leftovers:?}"
    );
}

#[test]
fn valid_checksum_entries_install() {
    if !supported() {
        return;
    }
    for (label, line_for) in [
        ("text mode", "{H}  {A}\n"),
        ("binary marker", "{H} *{A}\n"),
        ("uppercase hex and CRLF", "{U}  {A}\r\n"),
        (
            "other files listed too",
            "{O}  other-file.tar.gz\n{H}  {A}\n",
        ),
    ] {
        let (sandbox, digest) = sandbox("valid");
        let sums = line_for
            .replace("{H}", &digest)
            .replace("{U}", &digest.to_uppercase())
            .replace("{O}", &"1".repeat(64))
            .replace("{A}", ARCHIVE);
        let output = run_installer(&sandbox, &sums, &[], None);
        assert!(output.status.success(), "{label}: {}", stderr(&output));
        assert!(sandbox.installed(), "{label}");
    }
}

#[test]
fn missing_checksum_entry_fails_closed_with_a_diagnostic() {
    if !supported() {
        return;
    }
    let (sandbox, digest) = sandbox("missing");
    // Regression: a missing entry used to abort silently (pipefail + set -e).
    let output = run_installer(
        &sandbox,
        &format!("{digest}  unrelated.tar.gz\n"),
        &[],
        None,
    );
    assert_rejected(&sandbox, &output, "contains no checksum for");
    let output = run_installer(&sandbox, "", &[], None);
    assert_rejected(&sandbox, &output, "contains no checksum for");
}

#[test]
fn entries_for_other_file_names_never_match() {
    if !supported() {
        return;
    }
    // Regression: `grep "$ARCHIVE_NAME"` was an unanchored regex, so a line
    // for a *different* file (e.g. a signature) satisfied verification.
    for decoy in [
        format!("{ARCHIVE}.sig"),
        format!("prefix-{ARCHIVE}"),
        ARCHIVE.replace(".tar.gz", "Xtar.gz"),
        format!("dir/{ARCHIVE}"),
    ] {
        let (sandbox, digest) = sandbox("decoy");
        let output = run_installer(&sandbox, &format!("{digest}  {decoy}\n"), &[], None);
        assert_rejected(&sandbox, &output, "contains no checksum for");
    }
}

#[test]
fn duplicate_or_ambiguous_entries_are_refused() {
    if !supported() {
        return;
    }
    let (sandbox, digest) = sandbox("duplicate");
    let zeros = "0".repeat(64);
    let output = run_installer(
        &sandbox,
        &format!("{digest}  {ARCHIVE}\n{zeros}  {ARCHIVE}\n"),
        &[],
        None,
    );
    assert_rejected(&sandbox, &output, "refusing an ambiguous checksum");
    let output = run_installer(
        &sandbox,
        &format!("{digest}  {ARCHIVE}\n{digest}  {ARCHIVE}\n"),
        &[],
        None,
    );
    assert_rejected(&sandbox, &output, "refusing an ambiguous checksum");
}

#[test]
fn malformed_entries_and_wrong_digests_fail_closed() {
    if !supported() {
        return;
    }
    let (sandbox, digest) = sandbox("malformed");
    for sums in [
        format!("zz  {ARCHIVE}\n"),
        format!("{}  {ARCHIVE}\n", &digest[..63]),
        format!("{digest}{ARCHIVE}\n"),
        format!("{digest}  {ARCHIVE}\nthis line is garbage\n"),
    ] {
        let output = run_installer(&sandbox, &sums, &[], None);
        assert_rejected(&sandbox, &output, "malformed line in SHA256SUMS");
    }
    let output = run_installer(
        &sandbox,
        &format!("{}  {ARCHIVE}\n", "0".repeat(64)),
        &[],
        None,
    );
    assert_rejected(&sandbox, &output, "checksum mismatch");
}

#[test]
fn unsafe_inputs_are_rejected_before_downloading() {
    if !supported() {
        return;
    }
    let (sandbox, digest) = sandbox("inputs");
    let sums = format!("{digest}  {ARCHIVE}\n");
    let output = run_installer(
        &sandbox,
        &sums,
        &[],
        Some("http://example.invalid/releases"),
    );
    assert_rejected(&sandbox, &output, "must use https://");
    for version in ["0.2.1/../../evil", "v1.0.0;id", "latest", ""] {
        let output = run_installer(&sandbox, &sums, &["--version", version], None);
        assert_rejected(&sandbox, &output, "invalid version");
    }
    let output = run_installer(&sandbox, &sums, &["--version"], None);
    assert_rejected(&sandbox, &output, "requires a value");
}
