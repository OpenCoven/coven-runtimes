//! `conjure test` — conformance checks against the runtime binary.
//!
//! Validation (`conjure validate`) is pure and static. This command adds the
//! *dynamic* checks that need the real runtime present:
//!
//! - the declared `executable` resolves on `PATH`;
//! - it responds to a probe invocation (`--version` / `--help`) so we know the
//!   binary is actually runnable, not just present;
//! - declared flags — model, system-prompt, prompt binding, sandbox, stream,
//!   session continuity, and long-form launch-arg tokens — are plausibly
//!   referenced in the probe output (a soft warning, never a hard failure —
//!   CLIs vary).
//!
//! It never sends a real prompt or does any work; probes are read-only and
//! bounded. `--skip-binary` runs the static rules only (for CI without the
//! runtime installed).

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use clap::Args;
use coven_runtime_spec::{validate_manifest, RuntimeAdapter};

use super::load_manifest;

#[derive(Args)]
pub struct TestArgs {
    /// Path to the adapter manifest JSON.
    pub manifest: PathBuf,
    /// Skip the live binary probe; run only the static spec rules.
    #[arg(long)]
    pub skip_binary: bool,
    /// Probe flag to invoke (default tries `--version` then `--help`).
    #[arg(long)]
    pub probe_flag: Option<String>,
}

pub fn run(args: TestArgs) -> Result<()> {
    let manifest = load_manifest(&args.manifest)?;

    // Static rules first — a manifest that fails these can't be conformant.
    let errors = validate_manifest(&manifest);
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("✗ {e}");
        }
        bail!("static validation failed with {} problem(s)", errors.len());
    }
    println!("✓ static validation passed");

    if args.skip_binary {
        println!("· skipping binary probe (--skip-binary)");
        return Ok(());
    }

    let mut any_failed = false;
    for adapter in &manifest.adapters {
        match probe_adapter(adapter, args.probe_flag.as_deref()) {
            ProbeResult::Ok { probe, warnings } => {
                println!(
                    "✓ {} — `{}` responded to `{}`",
                    adapter.id, adapter.executable, probe
                );
                for w in warnings {
                    println!("  ⚠ {w}");
                }
            }
            ProbeResult::NotFound => {
                any_failed = true;
                eprintln!(
                    "✗ {} — executable `{}` not found on PATH ({})",
                    adapter.id, adapter.executable, adapter.install_hint
                );
            }
            ProbeResult::NotRunnable(msg) => {
                any_failed = true;
                eprintln!(
                    "✗ {} — `{}` did not run cleanly: {msg}",
                    adapter.id, adapter.executable
                );
            }
        }
    }

    if any_failed {
        bail!("conformance probe failed for one or more adapters");
    }
    println!("✓ conformance checks passed");
    Ok(())
}

enum ProbeResult {
    Ok {
        probe: String,
        warnings: Vec<String>,
    },
    NotFound,
    NotRunnable(String),
}

fn probe_adapter(adapter: &RuntimeAdapter, override_flag: Option<&str>) -> ProbeResult {
    probe_adapter_with_timeout(adapter, override_flag, Duration::from_secs(5))
}

fn probe_adapter_with_timeout(
    adapter: &RuntimeAdapter,
    override_flag: Option<&str>,
    timeout: Duration,
) -> ProbeResult {
    let flags: Vec<&str> = match override_flag {
        Some(f) => vec![f],
        None => vec!["--version", "--help"],
    };

    let mut last_err = String::new();
    for flag in &flags {
        match run_probe_command(&adapter.executable, flag, timeout) {
            Ok(output) => {
                // Any clean spawn+exit counts as runnable; many CLIs exit non-zero
                // on --version/--help, so we only require that it *ran*.
                let combined = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                let warnings = soft_flag_warnings(adapter, &combined);
                return ProbeResult::Ok {
                    probe: (*flag).to_string(),
                    warnings,
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ProbeResult::NotFound,
            Err(e) => last_err = e.to_string(),
        }
    }
    ProbeResult::NotRunnable(last_err)
}

fn run_probe_command(executable: &str, flag: &str, timeout: Duration) -> std::io::Result<Output> {
    let mut child = Command::new(executable)
        .arg(flag)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + timeout;

    loop {
        if child.try_wait()?.is_some() {
            return child.wait_with_output();
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("probe `{executable} {flag}` timed out after {timeout:?}"),
            ));
        }

        thread::sleep(Duration::from_millis(10));
    }
}

/// Soft checks: if the adapter declares a flag, note when the probe output
/// doesn't mention it. Covers every flag the manifest can declare — model,
/// system-prompt, prompt binding, sandbox, stream, continuity, and the
/// long-form tokens of every launch-arg list — since a typo in any of them
/// only surfaces at real session time otherwise. Never fails — CLIs don't
/// always list every flag in `--help`, and `--version` output is short.
///
/// Each distinct flag is checked (and warned about) once, labeled with the
/// first role it appears in: manifests commonly repeat a flag across launch
/// modes (e.g. Grok's `--single` as both prompt bindings), and the probe
/// output can't distinguish roles anyway.
fn soft_flag_warnings(adapter: &RuntimeAdapter, probe_output: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    let haystack = probe_output.to_lowercase();
    let mut seen: Vec<String> = Vec::new();
    let mut check = |flag: &str, what: &str| {
        let needle = flag.to_lowercase();
        if flag.is_empty() || seen.contains(&needle) {
            return;
        }
        seen.push(needle.clone());
        if !haystack.contains(&needle) {
            warnings.push(format!(
                "declared {what} flag `{flag}` not seen in probe output (verify manually)"
            ));
        }
    };
    if let Some(f) = &adapter.model_flag {
        check(f, "model");
    }
    if let Some(f) = &adapter.system_prompt_flag {
        check(f, "system-prompt");
    }
    if let Some(f) = &adapter.prompt_flag {
        check(f, "prompt");
    }
    if let Some(f) = &adapter.interactive_prompt_flag {
        check(f, "interactive-prompt");
    }
    for token in long_flags(&adapter.interactive_prompt_prefix_args) {
        check(token, "interactive launch");
    }
    for token in long_flags(&adapter.non_interactive_prompt_prefix_args) {
        check(token, "non-interactive launch");
    }
    if let Some(s) = &adapter.sandbox {
        for flag in s.probe_flags() {
            check(flag, "sandbox");
        }
    }
    if let Some(stream) = &adapter.stream_args {
        for token in long_flags(&stream.prefix_args) {
            check(token, "stream");
        }
        if let Some(f) = &stream.session_id_flag {
            check(f, "stream session-id");
        }
        if let Some(f) = &stream.resume_flag {
            check(f, "stream resume");
        }
    }
    if let Some(continuity) = &adapter.continuity_args {
        for token in long_flags(&continuity.init_prefix_args) {
            check(token, "continuity init");
        }
        for token in long_flags(&continuity.resume_prefix_args) {
            check(token, "continuity resume");
        }
        if let Some(f) = continuity.session_id_flag() {
            check(f, "continuity session-id");
        }
        if let Some(f) = continuity.resume_flag() {
            check(f, "continuity resume");
        }
    }
    warnings
}

/// Only long-form (`--x`) tokens of a launch-arg list are probe-checkable:
/// short flags and bare values like `stream-json` or `exec` would
/// false-positive against ordinary help text.
fn long_flags(args: &[String]) -> impl Iterator<Item = &str> {
    args.iter()
        .map(String::as_str)
        .filter(|t| t.starts_with("--") && t.len() > 2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use coven_runtime_spec::{Capabilities, SandboxMapping};

    fn adapter(exe: &str) -> RuntimeAdapter {
        RuntimeAdapter {
            id: "probe".into(),
            label: "Probe".into(),
            executable: exe.into(),
            interactive_prompt_prefix_args: vec![],
            non_interactive_prompt_prefix_args: vec!["exec".into()],
            install_hint: "install probe".into(),
            system_prompt_flag: None,
            model_flag: Some("--model".into()),
            model_arg_template: None,
            capabilities: Capabilities::BASELINE,
            sandbox: Some(SandboxMapping::Flag {
                flag: "--sandbox".into(),
                full: "full".into(),
                read_only: "read-only".into(),
            }),
            stream_args: None,
            prompt_flag: None,
            interactive_prompt_flag: None,
            continuity_args: None,
            version: None,
            homepage: None,
            description: None,
        }
    }

    #[test]
    fn probe_reports_not_found_for_missing_executable() {
        let a = adapter("definitely-not-a-real-binary-xyzzy-12345");
        assert!(matches!(probe_adapter(&a, None), ProbeResult::NotFound));
    }

    #[test]
    fn probe_runs_current_test_binary() {
        // The test harness is a platform-native executable that supports
        // `--help`, so this exercises a real probe without relying on a
        // Unix-only command such as `true`.
        let executable = std::env::current_exe().unwrap();
        let executable = executable.to_string_lossy();
        let a = adapter(&executable);
        match probe_adapter(&a, Some("--help")) {
            ProbeResult::Ok { .. } => {}
            other => panic!("expected Ok, got {:?}", DebugProbe(&other)),
        }
    }

    /// Unix-only: builds a small shell script that blocks longer than the
    /// probe timeout. Windows has no equivalent one-liner executable.
    #[cfg(unix)]
    #[test]
    fn probe_times_out_blocking_binary() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("blocks");
        fs::write(&script, "#!/bin/sh\nsleep 1\n").unwrap();
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();

        let a = adapter(script.to_str().unwrap());
        match probe_adapter_with_timeout(&a, Some("--version"), Duration::from_millis(50)) {
            ProbeResult::NotRunnable(msg) => assert!(msg.contains("timed out"), "{msg}"),
            other => panic!("expected NotRunnable timeout, got {:?}", DebugProbe(&other)),
        }
    }

    #[test]
    fn soft_warnings_flag_absent_declared_flags() {
        let a = adapter("probe");
        // Empty probe output => both declared flags (model, sandbox) warn.
        let warnings = soft_flag_warnings(&a, "");
        assert_eq!(warnings.len(), 2);
        // Output mentioning --model suppresses that one.
        let warnings = soft_flag_warnings(&a, "usage: --model <id>");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("--sandbox"));
    }

    #[test]
    fn soft_warnings_cover_system_prompt_and_stream_flags() {
        use coven_runtime_spec::StreamArgs;

        let mut a = adapter("true");
        a.system_prompt_flag = Some("--append-system-prompt".into());
        a.stream_args = Some(StreamArgs {
            // `-p` (short) and `stream-json` (bare value) must NOT be checked —
            // only long-form flags are meaningful against help text.
            prefix_args: vec!["-p".into(), "--output-format".into(), "stream-json".into()],
            session_id_flag: Some("--session-id".into()),
            resume_flag: Some("--resume".into()),
        });

        // Nothing mentioned => model + sandbox + system-prompt + 1 long stream
        // prefix flag + session-id + resume = 6 warnings.
        let warnings = soft_flag_warnings(&a, "");
        assert_eq!(warnings.len(), 6, "{warnings:?}");
        assert!(warnings
            .iter()
            .any(|w| w.contains("--append-system-prompt")));
        assert!(warnings.iter().any(|w| w.contains("--output-format")));
        assert!(warnings.iter().any(|w| w.contains("--session-id")));
        assert!(warnings.iter().any(|w| w.contains("--resume")));
        assert!(!warnings.iter().any(|w| w.contains("`-p`")), "{warnings:?}");
        assert!(
            !warnings.iter().any(|w| w.contains("stream-json")),
            "{warnings:?}"
        );

        // Help text mentioning all declared flags clears every warning.
        let all_mentioned = "usage: --model --sandbox --append-system-prompt \
                             --output-format --session-id --resume";
        assert!(soft_flag_warnings(&a, all_mentioned).is_empty());
    }

    /// Grok-shaped adapter: prompt bindings, launch prefix args, and
    /// continuity flags are covered, repeated flags are checked once, and
    /// bare values in launch-arg lists stay exempt.
    #[test]
    fn soft_warnings_cover_prompt_launch_and_continuity_flags() {
        use coven_runtime_spec::ContinuityArgs;

        let mut a = adapter("grok");
        a.system_prompt_flag = Some("--rules".into());
        // Same flag bound to both prompt roles — must warn once, not twice.
        a.prompt_flag = Some("--single".into());
        a.interactive_prompt_flag = Some("--single".into());
        let launch = vec![
            "--no-auto-update".into(),
            "--no-alt-screen".into(),
            "--output-format".into(),
            "plain".into(), // bare value: never checked
        ];
        a.interactive_prompt_prefix_args = launch.clone();
        a.non_interactive_prompt_prefix_args = launch.clone();
        a.sandbox = Some(SandboxMapping::Args {
            full_args: vec!["--permission-mode".into(), "bypassPermissions".into()],
            read_only_args: vec!["--sandbox".into(), "read-only".into()],
        });
        a.continuity_args = Some(ContinuityArgs {
            init_prefix_args: launch.clone(),
            resume_prefix_args: launch,
            session_id_flag: Some("--session-id".into()),
            resume_flag: Some("--resume".into()),
        });

        // Empty probe output: one warning per distinct flag —
        // --model, --rules, --single, --no-auto-update, --no-alt-screen,
        // --output-format, --permission-mode, --sandbox, --session-id,
        // --resume.
        let warnings = soft_flag_warnings(&a, "");
        assert_eq!(warnings.len(), 10, "{warnings:?}");
        assert!(warnings
            .iter()
            .any(|w| w.contains("prompt flag `--single`")));
        assert!(warnings
            .iter()
            .any(|w| w.contains("continuity session-id flag `--session-id`")));
        assert!(warnings
            .iter()
            .any(|w| w.contains("continuity resume flag `--resume`")));
        assert_eq!(
            warnings.iter().filter(|w| w.contains("--single")).count(),
            1,
            "repeated flags must be checked once: {warnings:?}"
        );
        assert!(
            !warnings.iter().any(|w| w.contains("`plain`")),
            "{warnings:?}"
        );

        // Mentioning the continuity + prompt flags clears exactly those.
        let warnings = soft_flag_warnings(&a, "usage: --single --session-id --resume");
        assert_eq!(warnings.len(), 7, "{warnings:?}");
        assert!(!warnings.iter().any(|w| w.contains("--single")));
        assert!(!warnings.iter().any(|w| w.contains("--session-id")));
        assert!(!warnings.iter().any(|w| w.contains("--resume")));
    }

    // Tiny helper so we can panic-print ProbeResult without a Debug impl on it.
    struct DebugProbe<'a>(&'a ProbeResult);
    impl std::fmt::Debug for DebugProbe<'_> {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self.0 {
                ProbeResult::Ok { probe, .. } => write!(f, "Ok({probe})"),
                ProbeResult::NotFound => write!(f, "NotFound"),
                ProbeResult::NotRunnable(m) => write!(f, "NotRunnable({m})"),
            }
        }
    }
}
