//! `conjure test` — conformance checks against the runtime binary.
//!
//! Validation (`conjure validate`) is pure and static. This command adds the
//! *dynamic* checks that need the real runtime present:
//!
//! - the declared `executable` resolves on `PATH`;
//! - it responds to a probe invocation (`--version` / `--help`) so we know the
//!   binary is actually runnable, not just present;
//! - declared model / sandbox / stream flags are plausibly referenced in the
//!   probe output (a soft warning, never a hard failure — CLIs vary).
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
/// doesn't mention it. Never fails — CLIs don't always list every flag in
/// `--help`, and `--version` output is short.
fn soft_flag_warnings(adapter: &RuntimeAdapter, probe_output: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    let haystack = probe_output.to_lowercase();
    let mut check = |flag: &str, what: &str| {
        if !flag.is_empty() && !haystack.contains(&flag.to_lowercase()) {
            warnings.push(format!(
                "declared {what} flag `{flag}` not seen in probe output (verify manually)"
            ));
        }
    };
    if let Some(f) = &adapter.model_flag {
        check(f, "model");
    }
    if let Some(s) = &adapter.sandbox {
        for flag in s.probe_flags() {
            check(flag, "sandbox");
        }
    }
    warnings
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
            event_protocol: None,
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
    fn probe_runs_real_binary_true() {
        // `true` exists on every unix and exits 0 with no output.
        let a = adapter("true");
        match probe_adapter(&a, Some("--version")) {
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
        let a = adapter("true");
        // Empty probe output => both declared flags (model, sandbox) warn.
        let warnings = soft_flag_warnings(&a, "");
        assert_eq!(warnings.len(), 2);
        // Output mentioning --model suppresses that one.
        let warnings = soft_flag_warnings(&a, "usage: --model <id>");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("--sandbox"));
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
