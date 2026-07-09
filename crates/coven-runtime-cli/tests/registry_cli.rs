//! Integration tests for `conjure registry`, exercising the real binary.
//!
//! The load-bearing one is `committed_index_matches_sources`: it is the drift
//! guard that fails CI if a source manifest under `registry/runtimes/` was
//! edited without regenerating the committed canonical index. The rest cover the
//! generator's contracts (idempotence, version immutability) and the
//! add/list/yank ergonomics on throwaway temp registries.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::tempdir;

fn conjure() -> Command {
    Command::new(env!("CARGO_BIN_EXE_conjure"))
}

/// The workspace root, where `registry/runtimes/` and the committed index live.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves")
}

/// A minimal, synthetic one-adapter manifest (id `aria`, never a built-in).
const ARIA_SRC: &str = r#"{
  "adapters": [{
    "id": "aria",
    "label": "Aria",
    "executable": "aria",
    "interactive_prompt_prefix_args": [],
    "non_interactive_prompt_prefix_args": ["exec"],
    "install_hint": "install aria and add it to PATH",
    "version": "1.0.0"
  }]
}"#;

/// Same id + version as `ARIA_SRC` but different content — must be rejected as an
/// immutable-version violation.
const ARIA_SRC_EDITED: &str = r#"{
  "adapters": [{
    "id": "aria",
    "label": "Aria (edited)",
    "executable": "aria",
    "interactive_prompt_prefix_args": [],
    "non_interactive_prompt_prefix_args": ["exec"],
    "install_hint": "install aria and add it to PATH",
    "version": "1.0.0"
  }]
}"#;

fn write_source(sources: &Path, id: &str, version: &str, body: &str) -> PathBuf {
    let dir = sources.join(id);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{version}.json"));
    fs::write(&path, body).unwrap();
    path
}

fn build(sources: &Path, out: &Path) -> Output {
    conjure()
        .args(["registry", "build", "--sources"])
        .arg(sources)
        .arg("--out")
        .arg(out)
        .output()
        .unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// DRIFT GUARD: the committed canonical index must equal a fresh rebuild from
/// the source manifests. Fails if someone edited `registry/runtimes/**` without
/// running `conjure registry build`.
#[test]
fn committed_index_matches_sources() {
    let out = conjure()
        .current_dir(workspace_root())
        .args(["registry", "check"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "canonical index is stale — run `conjure registry build` and commit it.\n{}",
        stderr(&out)
    );
}

#[test]
fn build_is_idempotent() {
    let dir = tempdir().unwrap();
    let sources = dir.path().join("registry/runtimes");
    let out = dir.path().join("canonical/index.json");
    write_source(&sources, "aria", "1.0.0", ARIA_SRC);

    let first = build(&sources, &out);
    assert!(first.status.success(), "{}", stderr(&first));
    let a = fs::read_to_string(&out).unwrap();

    let second = build(&sources, &out);
    assert!(second.status.success(), "{}", stderr(&second));
    let b = fs::read_to_string(&out).unwrap();

    // Second build preserves published_at, so the bytes are unchanged.
    assert_eq!(a, b);
    // `check` on the freshly built index passes.
    let checked = conjure()
        .args(["registry", "check", "--sources"])
        .arg(&sources)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(checked.status.success(), "{}", stderr(&checked));
}

#[test]
fn version_content_is_immutable() {
    let dir = tempdir().unwrap();
    let sources = dir.path().join("registry/runtimes");
    let out = dir.path().join("canonical/index.json");

    write_source(&sources, "aria", "1.0.0", ARIA_SRC);
    assert!(build(&sources, &out).status.success());

    // Change 1.0.0's content without bumping the version.
    write_source(&sources, "aria", "1.0.0", ARIA_SRC_EDITED);
    let rebuilt = build(&sources, &out);
    assert!(!rebuilt.status.success());
    assert!(
        stderr(&rebuilt).contains("already published with different content"),
        "unexpected error: {}",
        stderr(&rebuilt)
    );
}

#[test]
fn stale_index_fails_check() {
    let dir = tempdir().unwrap();
    let sources = dir.path().join("registry/runtimes");
    let out = dir.path().join("canonical/index.json");

    write_source(&sources, "aria", "1.0.0", ARIA_SRC);
    assert!(build(&sources, &out).status.success());

    // Add a brand-new runtime source but don't rebuild.
    write_source(&sources, "nova", "1.0.0", &ARIA_SRC.replace("aria", "nova"));
    let checked = conjure()
        .args(["registry", "check", "--sources"])
        .arg(&sources)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(!checked.status.success());
    assert!(stderr(&checked).contains("stale"), "{}", stderr(&checked));
}

#[test]
fn add_accepts_manifest_and_rebuilds() {
    let dir = tempdir().unwrap();
    let sources = dir.path().join("registry/runtimes");
    let out = dir.path().join("canonical/index.json");
    let manifest = dir.path().join("aria.json");
    fs::write(&manifest, ARIA_SRC).unwrap();

    let res = conjure()
        .args(["registry", "add"])
        .arg(&manifest)
        .arg("--sources")
        .arg(&sources)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(res.status.success(), "{}", stderr(&res));
    assert!(sources.join("aria/1.0.0.json").exists());
    assert!(out.exists());
}

#[test]
fn add_rejects_non_semver_version() {
    let dir = tempdir().unwrap();
    let sources = dir.path().join("registry/runtimes");
    let out = dir.path().join("canonical/index.json");
    let manifest = dir.path().join("badver.json");
    fs::write(&manifest, ARIA_SRC.replace("\"1.0.0\"", "\"not-semver\"")).unwrap();

    let res = conjure()
        .args(["registry", "add"])
        .arg(&manifest)
        .arg("--sources")
        .arg(&sources)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();

    assert!(!res.status.success());
    assert!(
        stderr(&res).contains("version `not-semver` is not valid semver"),
        "unexpected error: {}",
        stderr(&res)
    );
    assert!(!sources.join("aria/not-semver.json").exists());
}

#[test]
fn list_shows_accepted_runtimes() {
    let dir = tempdir().unwrap();
    let sources = dir.path().join("registry/runtimes");
    let out = dir.path().join("canonical/index.json");
    write_source(&sources, "aria", "1.0.0", ARIA_SRC);
    assert!(build(&sources, &out).status.success());

    let res = conjure()
        .args(["registry", "list", "--index"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(res.status.success(), "{}", stderr(&res));
    assert!(String::from_utf8_lossy(&res.stdout).contains("aria"));
}

#[test]
fn yank_persists_through_rebuild() {
    let dir = tempdir().unwrap();
    let sources = dir.path().join("registry/runtimes");
    let out = dir.path().join("canonical/index.json");
    write_source(&sources, "aria", "1.0.0", ARIA_SRC);
    assert!(build(&sources, &out).status.success());

    let yanked = conjure()
        .args(["registry", "yank", "aria", "1.0.0", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    assert!(yanked.status.success(), "{}", stderr(&yanked));
    assert!(fs::read_to_string(&out)
        .unwrap()
        .contains("\"yanked\": true"));

    // A rebuild must preserve the yank (it reads yanked from the committed index).
    assert!(build(&sources, &out).status.success());
    assert!(fs::read_to_string(&out)
        .unwrap()
        .contains("\"yanked\": true"));
}
