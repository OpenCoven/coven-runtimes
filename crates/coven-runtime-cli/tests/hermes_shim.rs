#![cfg(unix)]

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use tempfile::tempdir;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves")
}

#[test]
fn hermes_shim_preserves_model_before_remapping_the_prompt() {
    let dir = tempdir().expect("temporary directory");
    let fake_hermes = dir.path().join("hermes");
    fs::write(
        &fake_hermes,
        "#!/usr/bin/env bash\nprintf 'ARGV'; for arg in \"$@\"; do printf ' [%s]' \"$arg\"; done; printf '\\n'\n",
    )
    .expect("fake hermes written");
    fs::set_permissions(&fake_hermes, fs::Permissions::from_mode(0o755))
        .expect("fake hermes made executable");

    let path = format!(
        "{}:{}",
        dir.path().display(),
        env::var("PATH").expect("PATH is available")
    );
    let output = Command::new("bash")
        .arg(workspace_root().join("shims/hermes-coven"))
        .args([
            "chat",
            "--source",
            "coven",
            "-Q",
            "--model",
            "gpt-5.6-terra",
            "--",
            "hello from Coven",
        ])
        .env("PATH", path)
        .output()
        .expect("shim runs");

    assert!(output.status.success(), "{:?}", output);
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 argv"),
        "ARGV [chat] [--source] [coven] [-Q] [--model] [gpt-5.6-terra] [-q] [hello from Coven]\n"
    );
}
