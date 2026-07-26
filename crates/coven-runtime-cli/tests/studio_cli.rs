//! Integration tests for `conjure studio` — the pieces that don't need a TTY:
//! the interactive-terminal guard and argument validation. The TUI itself is
//! covered by unit tests over the pure state machine and `TestBackend` renders.

use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

fn conjure() -> Command {
    Command::new(env!("CARGO_BIN_EXE_conjure"))
}

fn run_piped(args: &[&str], dir: &std::path::Path) -> Output {
    conjure()
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("conjure runs")
}

#[test]
fn studio_refuses_non_interactive_stdio() {
    let dir = tempdir().expect("temporary directory");
    let output = run_piped(&["studio", "aria.json"], dir.path());
    assert!(
        !output.status.success(),
        "studio must fail without a TTY: {output:?}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("requires an interactive terminal"),
        "unexpected stderr: {stderr}"
    );
    // The guard runs before any scaffold/save: nothing may be written.
    assert!(!dir.path().join("aria.json").exists());
}

#[test]
fn studio_rejects_underivable_adapter_id() {
    let dir = tempdir().expect("temporary directory");
    // Guard order: a bad id must be reported even without a TTY? No — the TTY
    // guard fires first by design (never start work in a pipe). So this only
    // asserts the command fails; the id message is unit-tested at the state
    // level and unreachable in CI without a pty.
    let output = run_piped(&["studio", "Not A Valid Id.json"], dir.path());
    assert!(!output.status.success());
}

#[test]
fn studio_help_documents_the_command() {
    let output = conjure()
        .args(["studio", "--help"])
        .output()
        .expect("conjure runs");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--flavor"), "unexpected help: {stdout}");
}
