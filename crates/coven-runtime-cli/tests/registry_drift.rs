//! Repository-only drift guard for the canonical registry index.
//!
//! The crate archive intentionally omits the root-level registry sources, so
//! this integration target is excluded from publication but runs in workspace
//! CI to catch source edits that were not followed by `conjure registry build`.

use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves")
}

#[test]
fn committed_index_matches_sources() {
    let output = Command::new(env!("CARGO_BIN_EXE_conjure"))
        .current_dir(workspace_root())
        .args(["registry", "check"])
        .output()
        .expect("conjure registry check runs");
    assert!(
        output.status.success(),
        "canonical index is stale — run `conjure registry build` and commit it.\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
