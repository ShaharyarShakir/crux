# Crux Maintainer Release Checklist

**Target Version**: v0.2.1  
**Audience**: Crux Maintainers & Repository Owner  

This checklist must be executed and fully verified prior to creating release tags or publishing packages.

---

## Phase 1: Automated Code Quality Gates

Execute within the Nix development environment (`nix develop`):

- [ ] **Formatting Check**:
  ```bash
  cargo fmt --check
  ```
  *Requirement: Zero formatting divergence.*

- [ ] **Clippy Linter Verification**:
  ```bash
  cargo clippy --all-targets -- -D warnings
  ```
  *Requirement: Zero warnings, zero errors across library, binary, and test targets.*

- [ ] **Compilation Sanity Check**:
  ```bash
  cargo check --all-targets
  ```
  *Requirement: Clean build without warnings.*

- [ ] **Comprehensive Test Suite**:
  ```bash
  cargo test
  ```
  *Requirement: all automated tests passing.*

---

## Phase 2: Metadata & Licensing Audit

- [ ] **Verify `Cargo.toml` Metadata**:
  - `version = "0.2.1"` matches planned release.
  - `rust-version = "1.85.0"` accurately reflects MSRV.
  - `license = "MIT"` declared.
  - `description`, `repository`, `homepage`, `readme`, `keywords`, and `categories` are complete.
  - `exclude` list excludes internal `.github/`, `docs/`, `registry/`, and `tests/`.

- [ ] **Verify `LICENSE` File**:
  - `LICENSE` exists at repository root with correct copyright holder and year.

- [ ] **Verify `flake.nix` Package Target**:
  - `packages.${system}.default` builds cleanly with `nix build`.

---

## Phase 3: Distribution & Installation Verification

- [ ] **Release Compilation**:
  ```bash
  cargo build --release
  ./target/release/crux --version
  ./target/release/crux --help
  ```

- [ ] **Clean Cargo Install**:
  ```bash
  cargo install --path . --root /tmp/crux_test_install
  /tmp/crux_test_install/bin/crux --version
  rm -rf /tmp/crux_test_install
  ```

- [ ] **Package Archive Inspection**:
  ```bash
  cargo package --list --allow-dirty
  ```
  *Requirement: Verify that only necessary source files are packaged; target, cache, and test fixtures are excluded.*

---

## Phase 4: End-to-End System Validation

- [ ] **Full Consumer Workflow Test (Outside Repository)**:
  - `crux init` initializes consumer project.
  - `crux registry add` connects package registry.
  - `crux check` validates manifest.
  - `crux resolve` and `crux lock` generate reproducible lockfile.
  - `crux fetch` acquires sources and verifies SHA-256.
  - `crux build` compiles packages via CMake.
  - `crux install` stages and publishes package artifacts.
  - `crux integrate cmake` generates `CruxDependencies.cmake`.
  - External CMake consumer compiles, links, and runs successfully.

---

## Phase 5: Documentation Verification

- [ ] **README.md**:
  - Clear overview, quickstart, platform support, security note, and docs links.

- [ ] **Documentation Directory (`docs/`)**:
  - [docs/README.md](../README.md)
  - [docs/guides/installation.md](../guides/installation.md)
  - [docs/guides/quickstart.md](../guides/quickstart.md)
  - [docs/reference/cli-reference.md](../reference/cli-reference.md)
  - [docs/reference/security-model.md](../reference/security-model.md)
  - [docs/reference/architecture.md](../reference/architecture.md)
  - [docs/reports/release-readiness-audit.md](../reports/release-readiness-audit.md)

- [ ] **ROADMAP.md**:
  - Milestones M1–M16 and M20–M25 marked completed; M17–M19 remain planned in `ROADMAP.md`.
  - M24 was released in v0.2.0 and hardened in v0.2.1.
  - M25 v0.2.1 security/documentation hotfix recorded as completed.
  - M23 project import and M24 direct sources documented for v0.2.0.

---

## Phase 6: Git Tagging & Release Publication (Repository Owner Only)

*Note: Tagging and release publication are reserved for the repository owner. Automated agents are strictly prohibited from performing Git operations. The `v0.2.0` tag is already created, pushed, and published; do not move, retag, or rewrite it.*

- [ ] Ensure Git working tree is clean: `git status`.
- [ ] Create signed release tag:
  ```bash
  git tag -s v0.2.1 -m "Release v0.2.1: Security and documentation hotfix"
  ```
- [ ] Push commits and tag to remote:
  ```bash
  git push origin main --tags
  ```
- [ ] Publish crate to crates.io (when authorized):
  ```bash
  cargo publish
  ```
- [ ] Draft GitHub Release with detailed release notes and links to release artifacts.
