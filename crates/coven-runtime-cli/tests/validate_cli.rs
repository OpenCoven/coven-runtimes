//! Integration tests for `conjure validate`, exercising the real binary against
//! temp files. Guards the `--registry` wiring (blocker: registry indexes were
//! parsed as adapter manifests and always failed) and the manifest/registry
//! mode split.

use std::fs;
use std::process::Command;

use tempfile::tempdir;

/// Path to the freshly-built `conjure` binary Cargo hands integration tests.
fn conjure() -> Command {
    Command::new(env!("CARGO_BIN_EXE_conjure"))
}

const MANIFEST_JSON: &str = r#"{
  "adapters": [{
    "id": "hermes",
    "label": "Hermes Agent",
    "executable": "hermes",
    "interactive_prompt_prefix_args": ["chat", "--source", "coven", "-q"],
    "non_interactive_prompt_prefix_args": ["chat", "--source", "coven", "-Q", "-q"],
    "install_hint": "Install Hermes Agent and add it to PATH."
  }]
}"#;

const REGISTRY_JSON: &str = r#"{
  "format": "1",
  "runtimes": {
    "hermes": [{
      "version": "1.0.0",
      "adapter": {
        "id": "hermes",
        "label": "Hermes Agent",
        "executable": "hermes",
        "interactive_prompt_prefix_args": ["chat"],
        "non_interactive_prompt_prefix_args": ["chat", "-Q"],
        "install_hint": "Install Hermes Agent and add it to PATH."
      }
    }]
  }
}"#;

/// A registry index where an entry's adapter.id disagrees with its runtime key.
const REGISTRY_MISMATCH_JSON: &str = r#"{
  "runtimes": {
    "hermes": [{
      "version": "1.0.0",
      "adapter": {
        "id": "not-hermes",
        "label": "X",
        "executable": "x",
        "interactive_prompt_prefix_args": [],
        "non_interactive_prompt_prefix_args": ["exec"],
        "install_hint": "install"
      }
    }]
  }
}"#;

fn write(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn validates_a_plain_manifest() {
    let dir = tempdir().unwrap();
    let path = write(dir.path(), "manifest.json", MANIFEST_JSON);
    let out = conjure().arg("validate").arg(&path).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("valid"));
}

#[test]
fn validates_a_registry_index_with_flag() {
    let dir = tempdir().unwrap();
    let path = write(dir.path(), "index.json", REGISTRY_JSON);
    let out = conjure()
        .arg("validate")
        .arg("--registry")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("valid"));
    assert!(stdout.contains("runtime"));
}

#[test]
fn registry_index_without_flag_fails_loudly() {
    // The regression: a registry file run as a manifest must error clearly,
    // not silently "succeed" with zero adapters.
    let dir = tempdir().unwrap();
    let path = write(dir.path(), "index.json", REGISTRY_JSON);
    let out = conjure().arg("validate").arg(&path).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("looks like a registry index"), "{stderr}");
    assert!(stderr.contains("--registry"), "{stderr}");
}

#[test]
fn registry_id_key_mismatch_is_rejected() {
    let dir = tempdir().unwrap();
    let path = write(dir.path(), "index.json", REGISTRY_MISMATCH_JSON);
    let out = conjure()
        .arg("validate")
        .arg("--registry")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not match"));
}

#[test]
fn missing_file_errors() {
    let out = conjure()
        .arg("validate")
        .arg("/no/such/manifest.json")
        .output()
        .unwrap();
    assert!(!out.status.success());
}
