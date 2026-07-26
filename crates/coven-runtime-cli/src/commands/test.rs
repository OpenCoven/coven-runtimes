//! `conjure test` — conformance checks against the runtime binary.
//!
//! Validation (`conjure validate`) is pure and static. This command adds the
//! *dynamic* checks that need the real runtime present:
//!
//! - the declared `executable` resolves on `PATH`;
//! - it responds successfully to bounded leading-subcommand help or root
//!   help/version fallback, so we know the selected runtime and safe launch
//!   shape are runnable, not merely that a same-name binary is present;
//! - declared flags — model, system-prompt, prompt binding, sandbox, stream,
//!   session continuity, and long-form launch-arg tokens — are plausibly
//!   referenced in that help output (a soft warning, never a hard failure —
//!   CLIs vary).
//!
//! It never sends a real prompt or does any work; probes are read-only and
//! bounded. `--skip-binary` runs the static rules only (for CI without the
//! runtime installed).

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex, TryLockError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use clap::Args;
use coven_runtime_spec::{validate_manifest, RuntimeAdapter};

use super::load_manifest;

mod probe_child;

use probe_child::{ProbeChild, ProbeProcess, SpawnProbeError};

const PROBE_MODEL_PLACEHOLDER: &str = "conjure/probe-model";
const MAX_PROBE_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_DIAGNOSTIC_CHARS: usize = 1_024;
const READ_ONLY_PROBE_FLAGS: [&str; 4] = ["--help", "-h", "--version", "-V"];
/// A separate, short bound for terminating/reaping the process group or job
/// and finishing pipe readers after the main probe deadline.
const PROBE_CLEANUP_GRACE: Duration = Duration::from_millis(500);
const CLEANUP_RETRY_DELAY: Duration = Duration::from_millis(10);
const MAX_CLEANUP_ERRORS: usize = 8;

#[derive(Args)]
pub struct TestArgs {
    /// Path to the adapter manifest JSON.
    pub manifest: PathBuf,
    /// Skip the live binary probe; run only the static spec rules.
    #[arg(long)]
    pub skip_binary: bool,
    /// Read-only probe flag: --help, -h, --version, or -V. It follows only a
    /// safe leading subcommand; other recipes are probed at the root.
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
                println!("✓ {} — probe `{probe}` succeeded", adapter.id);
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

/// Outcome of probing one adapter's binary, in the shape `conjure studio`
/// renders. A thin owned view over [`ProbeResult`] so the TUI shares the exact
/// probe (and soft-warning rules) `conjure test` enforces.
pub(crate) enum ProbeReport {
    /// Binary resolved and ran; `probe` is the safe invocation that worked.
    Ok {
        probe: String,
        warnings: Vec<String>,
    },
    /// Executable not found on PATH.
    NotFound,
    /// Found but did not run cleanly within the probe bounds.
    NotRunnable(String),
}

/// Probe an adapter's declared executable exactly as `conjure test` does:
/// bounded model + leading-subcommand help (or root help/version fallback)
/// plus the soft flag warnings.
pub(crate) fn probe_adapter_report(adapter: &RuntimeAdapter) -> ProbeReport {
    match probe_adapter(adapter, None) {
        ProbeResult::Ok { probe, warnings } => ProbeReport::Ok { probe, warnings },
        ProbeResult::NotFound => ProbeReport::NotFound,
        ProbeResult::NotRunnable(msg) => ProbeReport::NotRunnable(msg),
    }
}

fn probe_adapter(adapter: &RuntimeAdapter, override_flag: Option<&str>) -> ProbeResult {
    probe_adapter_with_timeout(adapter, override_flag, Duration::from_secs(5))
}

fn probe_adapter_with_timeout(
    adapter: &RuntimeAdapter,
    override_flag: Option<&str>,
    timeout: Duration,
) -> ProbeResult {
    let override_flag = if let Some(flag) = override_flag {
        let flag = flag.trim();
        if !READ_ONLY_PROBE_FLAGS.contains(&flag) {
            return ProbeResult::NotRunnable(format!(
                "probe flag `{flag}` is not an approved read-only flag \
                 (allowed: --help, -h, --version, -V)"
            ));
        }
        Some(flag)
    } else {
        None
    };
    let mut last_err = String::new();
    for invocation in probe_invocations(adapter, override_flag) {
        let probe = display_invocation(&adapter.executable, &invocation.args);
        match run_probe_command(&adapter.executable, &invocation.args, timeout) {
            Ok(output) => {
                if !output.status.success() {
                    last_err = failed_probe_diagnostic(&probe, &output);
                    continue;
                }
                let combined = output.combined();
                if let Some(required_usage) = &invocation.required_usage {
                    if !has_usage_signature(&combined, required_usage) {
                        last_err = format!(
                            "probe `{probe}` exited successfully but help output did not identify \
                             the declared subcommand (expected `Usage: {required_usage}`); {}; {}",
                            diagnostic_context("stdout", &output.stdout),
                            diagnostic_context("stderr", &output.stderr)
                        );
                        continue;
                    }
                }
                let warnings = soft_flag_warnings(adapter, &combined);
                return ProbeResult::Ok { probe, warnings };
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ProbeResult::NotFound,
            Err(e) => last_err = e.to_string(),
        }
    }
    ProbeResult::NotRunnable(last_err)
}

struct ProbeInvocation {
    args: Vec<String>,
    required_usage: Option<String>,
}

fn probe_invocations(
    adapter: &RuntimeAdapter,
    override_flag: Option<&str>,
) -> Vec<ProbeInvocation> {
    if let Some(subcommand) = leading_subcommand(&adapter.non_interactive_prompt_prefix_args) {
        let mut args = probe_model_tokens(adapter);
        args.push(subcommand.to_string());
        args.push(override_flag.unwrap_or("--help").to_string());
        let required_usage = executable_basename(&adapter.executable)
            .map(|executable| format!("{executable} {subcommand}"));
        return vec![ProbeInvocation {
            args,
            required_usage,
        }];
    }

    match override_flag {
        Some(flag) => vec![ProbeInvocation {
            args: vec![flag.to_string()],
            required_usage: None,
        }],
        None => ["--help", "--version"]
            .into_iter()
            .map(|flag| ProbeInvocation {
                args: vec![flag.to_string()],
                required_usage: None,
            })
            .collect(),
    }
}

fn probe_model_tokens(adapter: &RuntimeAdapter) -> Vec<String> {
    if let Some(template) = &adapter.model_arg_template {
        return template
            .split_whitespace()
            .map(|token| token.replace("{model}", PROBE_MODEL_PLACEHOLDER))
            .collect();
    }
    adapter
        .model_flag
        .as_ref()
        .map(|flag| vec![flag.clone(), PROBE_MODEL_PLACEHOLDER.into()])
        .unwrap_or_default()
}

fn leading_subcommand(args: &[String]) -> Option<&str> {
    args.first()
        .map(String::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty() && !token.starts_with('-'))
}

fn executable_basename(executable: &str) -> Option<String> {
    let name = Path::new(executable).file_name()?.to_string_lossy();
    let mut name = name.into_owned();
    if name.to_ascii_lowercase().ends_with(".exe") {
        name.truncate(name.len() - 4);
    }
    Some(name)
}

fn has_usage_signature(output: &str, required_usage: &str) -> bool {
    let required_usage = normalized_whitespace(required_usage).to_lowercase();
    output.lines().any(|line| {
        let line = normalized_whitespace(line).to_lowercase();
        let candidate = line
            .strip_prefix("usage: ")
            .or_else(|| line.strip_prefix("usage "))
            .unwrap_or(&line);
        candidate
            .strip_prefix(&required_usage)
            .is_some_and(|remainder| {
                matches!(remainder.chars().next(), None | Some(' ' | '[' | '<'))
            })
    })
}

fn normalized_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn display_invocation(executable: &str, args: &[String]) -> String {
    std::iter::once(executable)
        .chain(args.iter().map(String::as_str))
        .map(|token| {
            if token.is_empty() || token.chars().any(char::is_whitespace) {
                format!("{token:?}")
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

struct CapturedStream {
    text: String,
    truncated: bool,
}

struct ProbeOutput {
    status: ExitStatus,
    stdout: CapturedStream,
    stderr: CapturedStream,
}

impl ProbeOutput {
    fn combined(&self) -> String {
        format!("{}\n{}", self.stdout.text, self.stderr.text)
    }
}

fn capture_stream(mut stream: impl Read) -> io::Result<CapturedStream> {
    let mut bytes = Vec::with_capacity(MAX_PROBE_OUTPUT_BYTES);
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = MAX_PROBE_OUTPUT_BYTES.saturating_sub(bytes.len());
        let retained = remaining.min(read);
        bytes.extend_from_slice(&buffer[..retained]);
        truncated |= retained < read;
    }
    Ok(CapturedStream {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
    })
}

struct BackgroundTask<T> {
    receiver: Receiver<T>,
    handle: Option<JoinHandle<()>>,
    value_received: bool,
    label: &'static str,
    workers: Arc<WorkerSet>,
}

trait ThreadSpawner {
    fn spawn(
        &self,
        name: String,
        operation: Box<dyn FnOnce() + Send + 'static>,
    ) -> io::Result<JoinHandle<()>>;
}

struct SystemThreadSpawner;

impl ThreadSpawner for SystemThreadSpawner {
    fn spawn(
        &self,
        name: String,
        operation: Box<dyn FnOnce() + Send + 'static>,
    ) -> io::Result<JoinHandle<()>> {
        thread::Builder::new().name(name).spawn(operation)
    }
}

struct RetainedWorker {
    label: &'static str,
    handle: JoinHandle<()>,
}

static PROCESS_LIFETIME_WORKERS: Mutex<Vec<RetainedWorker>> = Mutex::new(Vec::new());

fn reap_process_lifetime_workers() {
    let mut workers = PROCESS_LIFETIME_WORKERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut index = 0;
    while index < workers.len() {
        if workers[index].handle.is_finished() {
            let worker = workers.swap_remove(index);
            let _ = worker.handle.join();
        } else {
            index += 1;
        }
    }
}

#[cfg(all(test, unix))]
fn process_lifetime_worker_count(label: &'static str) -> usize {
    PROCESS_LIFETIME_WORKERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .filter(|worker| worker.label == label)
        .count()
}

#[derive(Default)]
struct WorkerSet {
    retained: Mutex<Vec<RetainedWorker>>,
}

impl WorkerSet {
    fn retain(&self, worker: RetainedWorker) {
        self.retained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(worker);
    }

    fn preflight(&self) -> io::Result<()> {
        let mut workers = match self.retained.try_lock() {
            Ok(workers) => workers,
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "previous probe workers are being checked",
                ));
            }
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        let mut errors = Vec::new();
        let mut index = 0;
        while index < workers.len() {
            if workers[index].handle.is_finished() {
                let worker = workers.swap_remove(index);
                if worker.handle.join().is_err() {
                    errors.push(format!("probe {} task panicked", worker.label));
                }
            } else {
                index += 1;
            }
        }

        if !workers.is_empty() {
            let labels = workers
                .iter()
                .map(|worker| worker.label)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                format!("previous probe workers are still running: {labels}"),
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(io::Error::other(errors.join("; ")))
        }
    }
}

impl Drop for WorkerSet {
    fn drop(&mut self) {
        let workers = self
            .retained
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut unfinished = Vec::new();
        for worker in workers.drain(..) {
            if worker.handle.is_finished() {
                let _ = worker.handle.join();
            } else {
                unfinished.push(worker);
            }
        }
        if !unfinished.is_empty() {
            reap_process_lifetime_workers();
            PROCESS_LIFETIME_WORKERS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend(unfinished);
        }
    }
}

impl<T: Send + 'static> BackgroundTask<T> {
    fn spawn_with(
        label: &'static str,
        operation: impl FnOnce() -> T + Send + 'static,
        spawner: &dyn ThreadSpawner,
        workers: Arc<WorkerSet>,
    ) -> io::Result<Self> {
        Self::spawn_with_guard(label, operation, spawner, workers, ())
    }

    fn spawn_with_guard(
        label: &'static str,
        operation: impl FnOnce() -> T + Send + 'static,
        spawner: &dyn ThreadSpawner,
        workers: Arc<WorkerSet>,
        completion_guard: impl Send + 'static,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let handle = spawner.spawn(
            format!("conjure probe {label}"),
            Box::new(move || {
                let result = operation();
                let _ = sender.send(result);
                drop(completion_guard);
            }),
        )?;
        Ok(Self {
            receiver,
            handle: Some(handle),
            value_received: false,
            label,
            workers,
        })
    }

    fn poll(&mut self) -> io::Result<Option<T>> {
        if self.value_received {
            self.join_if_finished()?;
            return Ok(None);
        }

        match self.receiver.try_recv() {
            Ok(value) => {
                self.value_received = true;
                self.join_if_finished()?;
                Ok(Some(value))
            }
            Err(TryRecvError::Empty) => {
                self.join_if_finished()?;
                Ok(None)
            }
            Err(TryRecvError::Disconnected) => {
                self.value_received = true;
                self.join_if_finished()?;
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("probe {} task disconnected before reporting", self.label),
                ))
            }
        }
    }

    fn join_if_finished(&mut self) -> io::Result<()> {
        let finished = self
            .handle
            .as_ref()
            .is_some_and(|handle| handle.is_finished());
        if finished {
            let handle = self.handle.take().ok_or_else(|| {
                io::Error::other(format!(
                    "probe {} task lost its finished worker handle",
                    self.label
                ))
            })?;
            handle
                .join()
                .map_err(|_| io::Error::other(format!("probe {} task panicked", self.label)))?;
        }
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.value_received && self.handle.is_none()
    }
}

impl<T> Drop for BackgroundTask<T> {
    fn drop(&mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        self.workers.retain(RetainedWorker {
            label: self.label,
            handle,
        });
    }
}

fn capture_stream_async(
    stream: impl Read + Send + 'static,
    label: &'static str,
    spawner: &dyn ThreadSpawner,
    workers: Arc<WorkerSet>,
) -> io::Result<BackgroundTask<io::Result<CapturedStream>>> {
    BackgroundTask::spawn_with(label, move || capture_stream(stream), spawner, workers)
}

fn poll_capture(
    reader: &mut BackgroundTask<io::Result<CapturedStream>>,
    captured: &mut Option<CapturedStream>,
) -> io::Result<()> {
    if let Some(result) = reader.poll()? {
        if captured.is_some() {
            return Err(io::Error::other(format!(
                "probe {} task reported output more than once",
                reader.label
            )));
        }
        *captured = Some(result?);
    }
    Ok(())
}

fn group_already_exited(error: &io::Error) -> bool {
    if matches!(
        error.kind(),
        io::ErrorKind::InvalidInput | io::ErrorKind::NotFound
    ) {
        return true;
    }

    #[cfg(unix)]
    {
        // `killpg` reports ESRCH once the process group is already empty.
        error.raw_os_error() == Some(3)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

struct CleanupOutcome {
    status: ExitStatus,
    stdout: Option<CapturedStream>,
    stderr: Option<CapturedStream>,
}

type ProbeTerminator = dyn FnMut(&mut ProbeChild) -> io::Result<()> + Send;

#[derive(Default)]
struct CleanupProgress {
    errors: Mutex<Vec<String>>,
}

impl CleanupProgress {
    fn record(&self, detail: String) {
        let mut errors = self
            .errors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if errors.len() < MAX_CLEANUP_ERRORS && !errors.contains(&detail) {
            errors.push(detail);
        }
    }

    fn detail(&self) -> String {
        self.errors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .join("; ")
    }
}

struct CleanupWork {
    child: ProbeChild,
    stdout_reader: Option<BackgroundTask<io::Result<CapturedStream>>>,
    stderr_reader: Option<BackgroundTask<io::Result<CapturedStream>>>,
    leader_exited: bool,
    status: Option<ExitStatus>,
    stdout: Option<CapturedStream>,
    stderr: Option<CapturedStream>,
    errors: Vec<String>,
    omitted_errors: usize,
    group_exited: bool,
    terminate: Box<ProbeTerminator>,
    progress: Option<Arc<CleanupProgress>>,
    #[cfg(unix)]
    deferred_group_permission: Option<String>,
    permission_denied: bool,
}

impl CleanupWork {
    fn record_error(&mut self, detail: impl Into<String>) {
        let detail = detail.into();
        if self.errors.iter().any(|error| error == &detail) {
            return;
        }
        if self.errors.len() >= MAX_CLEANUP_ERRORS {
            self.omitted_errors = self.omitted_errors.saturating_add(1);
            return;
        }
        let detail: String = detail.chars().take(MAX_DIAGNOSTIC_CHARS).collect();
        self.errors.push(detail.clone());
        if let Some(progress) = &self.progress {
            progress.record(detail);
        }
    }

    fn record_permission_error(&mut self, detail: impl Into<String>) {
        self.permission_denied = true;
        self.record_error(detail);
    }

    #[cfg(unix)]
    fn defer_group_permission(&mut self, context: &str, error: &io::Error) {
        let detail = format!("{context}: {error}");
        if self.deferred_group_permission.is_none() {
            self.deferred_group_permission = Some(detail.clone());
        }
        if let Some(progress) = &self.progress {
            progress.record(format!("{detail}; awaiting post-reap group classification"));
        }
    }

    fn error_detail(&self) -> String {
        let mut detail = self.errors.join("; ");
        if self.omitted_errors > 0 {
            if !detail.is_empty() {
                detail.push_str("; ");
            }
            detail.push_str(&format!(
                "{} additional cleanup error(s) omitted",
                self.omitted_errors
            ));
        }
        detail
    }
}

fn poll_cleanup_capture(
    reader: &mut Option<BackgroundTask<io::Result<CapturedStream>>>,
    captured: &mut Option<CapturedStream>,
    new_error: &mut Option<String>,
    label: &'static str,
) {
    let Some(task) = reader.as_mut() else {
        return;
    };
    if let Err(error) = poll_capture(task, captured) {
        *new_error = Some(format!("failed to finish {label} capture: {error}"));
        *reader = None;
        return;
    }
    if task.is_complete() {
        if captured.is_none() {
            *new_error = Some(format!("probe {label} task completed without output"));
        }
        *reader = None;
    }
}

fn finish_cleanup_work(mut work: CleanupWork) -> io::Result<CleanupOutcome> {
    loop {
        #[cfg(unix)]
        let mut termination_permission_denied = false;
        if !work.group_exited {
            match work.child.try_wait_group() {
                Ok(exited) => work.group_exited = exited,
                Err(error) => {
                    #[cfg(unix)]
                    if work.child.consumed_group_permission(&error) {
                        work.defer_group_permission(
                            "permission denied while polling probe process group",
                            &error,
                        );
                        work.group_exited = true;
                    } else if error.kind() == io::ErrorKind::PermissionDenied {
                        work.record_permission_error(format!(
                            "failed to poll probe process group/job completion: {error}"
                        ));
                    } else {
                        work.record_error(format!(
                            "failed to poll probe process group/job completion: {error}"
                        ));
                    }
                    #[cfg(not(unix))]
                    if error.kind() == io::ErrorKind::PermissionDenied {
                        work.record_permission_error(format!(
                            "failed to poll probe process group/job completion: {error}"
                        ));
                    } else {
                        work.record_error(format!(
                            "failed to poll probe process group/job completion: {error}"
                        ));
                    }
                }
            }
        }
        if !work.group_exited {
            let termination = {
                let terminate = &mut work.terminate;
                terminate(&mut work.child)
            };
            if let Err(error) = termination {
                if !group_already_exited(&error) {
                    #[cfg(unix)]
                    if work.child.consumed_group_permission(&error) {
                        work.defer_group_permission(
                            "permission denied while terminating probe process group",
                            &error,
                        );
                        work.group_exited = true;
                    } else {
                        termination_permission_denied =
                            error.kind() == io::ErrorKind::PermissionDenied;
                        if termination_permission_denied {
                            work.record_permission_error(format!(
                                "failed to terminate probe process group/job: {error}"
                            ));
                        } else {
                            work.record_error(format!(
                                "failed to terminate probe process group/job: {error}"
                            ));
                        }
                    }
                    #[cfg(not(unix))]
                    if error.kind() == io::ErrorKind::PermissionDenied {
                        work.record_permission_error(format!(
                            "failed to terminate probe process group/job: {error}"
                        ));
                    } else {
                        work.record_error(format!(
                            "failed to terminate probe process group/job: {error}"
                        ));
                    }
                }
            }
        }
        if !work.leader_exited {
            match work.child.try_wait_leader() {
                Ok(exited) => work.leader_exited = exited,
                Err(error) => work.record_error(format!(
                    "failed to observe probe process group/job leader: {error}"
                )),
            }
        }
        #[cfg(unix)]
        if termination_permission_denied && work.leader_exited {
            // Once the leader is waitable, a permission-denied terminal
            // decision means this owner can no longer safely target the
            // numeric PGID. Consume it before reaping the identity anchor.
            work.child.abandon_group_target();
            work.group_exited = true;
        }
        if work.group_exited && work.leader_exited && work.status.is_none() {
            match work.child.reap_leader() {
                Ok(status) => work.status = status,
                Err(error) => work.record_error(format!(
                    "failed to reap probe process group/job leader: {error}"
                )),
            }
        }
        #[cfg(unix)]
        if work.status.is_some() {
            if let Some(permission) = work.deferred_group_permission.take() {
                match work.child.post_reap_group_exists() {
                    Ok(false) => {}
                    Ok(true) => work.record_permission_error(format!(
                        "{permission}; process group still exists after reaping its leader"
                    )),
                    Err(error) => work.record_permission_error(format!(
                        "{permission}; failed post-reap process-group classification: {error}"
                    )),
                }
            }
        }

        let mut stdout_error = None;
        poll_cleanup_capture(
            &mut work.stdout_reader,
            &mut work.stdout,
            &mut stdout_error,
            "stdout reader",
        );
        if let Some(error) = stdout_error {
            work.record_error(error);
        }
        let mut stderr_error = None;
        poll_cleanup_capture(
            &mut work.stderr_reader,
            &mut work.stderr,
            &mut stderr_error,
            "stderr reader",
        );
        if let Some(error) = stderr_error {
            work.record_error(error);
        }

        if work.group_exited
            && work.status.is_some()
            && work.stdout_reader.is_none()
            && work.stderr_reader.is_none()
        {
            break;
        }
        thread::sleep(CLEANUP_RETRY_DELAY);
    }

    if !work.errors.is_empty() {
        let kind = if work.permission_denied {
            io::ErrorKind::PermissionDenied
        } else {
            io::ErrorKind::Other
        };
        return Err(io::Error::new(kind, work.error_detail()));
    }

    Ok(CleanupOutcome {
        status: work.status.ok_or_else(|| {
            io::Error::other("probe process group/job did not report a leader status")
        })?,
        stdout: work.stdout,
        stderr: work.stderr,
    })
}

enum CleanupCommand {
    Run(Box<CleanupWork>),
    Cancel,
}

struct CleanupSubmissionError {
    error: io::Error,
    work: Box<CleanupWork>,
}

struct CleanupSupervisor {
    sender: Option<SyncSender<CleanupCommand>>,
    task: BackgroundTask<io::Result<Option<CleanupOutcome>>>,
    completion: Option<io::Result<Option<CleanupOutcome>>>,
    caller_lease: Option<AdmissionLease>,
    progress: Arc<CleanupProgress>,
}

impl CleanupSupervisor {
    fn spawn_with(
        spawner: &dyn ThreadSpawner,
        lease: &AdmissionLease,
        workers: Arc<WorkerSet>,
    ) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker_lease = lease.clone();
        let progress = Arc::new(CleanupProgress::default());
        let task = BackgroundTask::spawn_with_guard(
            "cleanup supervisor",
            move || match receiver.recv() {
                Ok(CleanupCommand::Run(work)) => finish_cleanup_work(*work).map(Some),
                Ok(CleanupCommand::Cancel) => Ok(None),
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "probe cleanup supervisor command channel disconnected",
                )),
            },
            spawner,
            workers,
            worker_lease,
        )?;
        Ok(Self {
            sender: Some(sender),
            task,
            completion: None,
            caller_lease: Some(lease.clone()),
            progress,
        })
    }

    fn submit(&mut self, mut work: Box<CleanupWork>) -> Result<(), CleanupSubmissionError> {
        let Some(sender) = self.sender.take() else {
            return Err(CleanupSubmissionError {
                error: io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "probe cleanup supervisor was already submitted",
                ),
                work,
            });
        };
        for error in &work.errors {
            self.progress.record(error.clone());
        }
        work.progress = Some(Arc::clone(&self.progress));
        match sender.send(CleanupCommand::Run(work)) {
            Ok(()) => {
                drop(self.caller_lease.take());
                Ok(())
            }
            Err(mpsc::SendError(CleanupCommand::Run(work))) => Err(CleanupSubmissionError {
                error: io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "probe cleanup supervisor disconnected before taking ownership",
                ),
                work,
            }),
            Err(mpsc::SendError(CleanupCommand::Cancel)) => {
                unreachable!("submitted cleanup command changed variants")
            }
        }
    }

    fn cancel(&mut self) -> io::Result<()> {
        let Some(sender) = self.sender.take() else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "probe cleanup supervisor was already submitted",
            ));
        };
        sender
            .send(CleanupCommand::Cancel)
            .map(|()| drop(self.caller_lease.take()))
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "probe cleanup supervisor disconnected before cancellation",
                )
            })
    }

    fn take_caller_lease(&mut self) -> io::Result<AdmissionLease> {
        self.caller_lease.take().ok_or_else(|| {
            io::Error::other("cleanup supervisor no longer owns its caller admission lease")
        })
    }

    fn progress_detail(&self) -> String {
        self.progress.detail()
    }

    fn poll(&mut self) -> io::Result<()> {
        if self.completion.is_none() {
            if let Some(completion) = self.task.poll()? {
                self.completion = Some(completion);
            }
        } else if self.task.poll()?.is_some() {
            return Err(io::Error::other(
                "probe cleanup supervisor reported more than once",
            ));
        }
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.completion.is_some() && self.task.is_complete()
    }

    fn take_completion(mut self) -> io::Result<Option<CleanupOutcome>> {
        if !self.is_complete() {
            return Err(io::Error::other("probe cleanup supervisor is not complete"));
        }
        self.completion
            .take()
            .ok_or_else(|| io::Error::other("probe cleanup supervisor did not report"))?
    }
}

enum DurableCleanup {
    Running(CleanupSupervisor),
    Stalled {
        work: Box<CleanupWork>,
        cause: String,
    },
}

struct CleanupRegistry {
    inner: Arc<CleanupRegistryInner>,
}

struct CleanupRegistryInner {
    active: AtomicBool,
    state: Mutex<CleanupRegistryState>,
    workers: Arc<WorkerSet>,
}

#[derive(Default)]
struct CleanupRegistryState {
    retained: Option<DurableCleanup>,
}

#[derive(Clone)]
struct AdmissionLease {
    _release: Arc<AdmissionRelease>,
}

struct AdmissionRelease {
    registry: Weak<CleanupRegistryInner>,
}

impl Drop for AdmissionRelease {
    fn drop(&mut self) {
        if let Some(registry) = self.registry.upgrade() {
            registry.active.store(false, Ordering::Release);
        }
    }
}

impl CleanupRegistry {
    fn new() -> Self {
        Self {
            inner: Arc::new(CleanupRegistryInner {
                active: AtomicBool::new(false),
                state: Mutex::new(CleanupRegistryState::default()),
                workers: Arc::new(WorkerSet::default()),
            }),
        }
    }

    fn acquire(&self) -> io::Result<AdmissionLease> {
        self.inner
            .active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "previous probe cleanup is still running",
                )
            })?;
        Ok(AdmissionLease {
            _release: Arc::new(AdmissionRelease {
                registry: Arc::downgrade(&self.inner),
            }),
        })
    }

    fn workers(&self) -> Arc<WorkerSet> {
        Arc::clone(&self.inner.workers)
    }

    fn retain(&self, cleanup: DurableCleanup) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(
            state.retained.is_none(),
            "atomic probe admission allowed multiple retained cleanups"
        );
        state.retained = Some(cleanup);
    }

    fn preflight(&self, lease: &AdmissionLease, spawner: &dyn ThreadSpawner) -> io::Result<()> {
        self.inner.workers.preflight()?;
        let retained = {
            let mut state = match self.inner.state.try_lock() {
                Ok(state) => state,
                Err(TryLockError::WouldBlock) => {
                    return Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "previous probe cleanup is being checked",
                    ));
                }
                Err(TryLockError::Poisoned(error)) => error.into_inner(),
            };
            state.retained.take()
        };
        let Some(retained) = retained else {
            return Ok(());
        };

        match retained {
            DurableCleanup::Running(mut supervisor) => match supervisor.poll() {
                Ok(()) if supervisor.is_complete() => match supervisor.take_completion()? {
                    Some(_) => Ok(()),
                    None => Err(io::Error::other("cleanup supervisor was cancelled")),
                },
                Ok(()) => {
                    self.retain(DurableCleanup::Running(supervisor));
                    Err(io::Error::new(
                        io::ErrorKind::WouldBlock,
                        "previous probe cleanup is still running",
                    ))
                }
                Err(error) if supervisor.task.is_complete() => Err(io::Error::new(
                    error.kind(),
                    format!("previous probe cleanup failed: {error}"),
                )),
                Err(error) => {
                    let detail = format!(
                        "previous probe cleanup is still running; supervisor poll failed: {error}"
                    );
                    self.retain(DurableCleanup::Running(supervisor));
                    Err(io::Error::new(io::ErrorKind::WouldBlock, detail))
                }
            },
            DurableCleanup::Stalled { mut work, cause } => {
                let mut supervisor =
                    match CleanupSupervisor::spawn_with(spawner, lease, self.workers()) {
                        Ok(supervisor) => supervisor,
                        Err(error) => {
                            let cause =
                                format!("{cause}; cleanup supervisor restart failed: {error}");
                            work.record_error(format!(
                                "stalled cleanup supervisor restart failed: {error}"
                            ));
                            self.retain(DurableCleanup::Stalled { work, cause });
                            return Err(io::Error::new(
                                error.kind(),
                                format!(
                                    "previous probe cleanup restart failed: {error}; \
                                     cleanup ownership remains durably retained"
                                ),
                            ));
                        }
                    };
                work.record_error(format!("stalled cleanup recovery: {cause}"));
                if let Err(submission) = supervisor.submit(work) {
                    let CleanupSubmissionError {
                        error,
                        work: mut returned_work,
                    } = submission;
                    drop(supervisor.take_caller_lease());
                    drop(supervisor);
                    returned_work.record_error(format!(
                        "replacement cleanup supervisor submission failed: {error}"
                    ));
                    let cause =
                        format!("{cause}; cleanup supervisor restart submission failed: {error}");
                    self.retain(DurableCleanup::Stalled {
                        work: returned_work,
                        cause,
                    });
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "previous probe cleanup restart submission failed: {error}; \
                             cleanup ownership remains durably retained"
                        ),
                    ));
                }
                self.retain(DurableCleanup::Running(supervisor));
                Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    format!(
                        "previous probe cleanup was restarted: {cause}; \
                         current probe was not spawned"
                    ),
                ))
            }
        }
    }
}

#[cfg(not(test))]
static DURABLE_CLEANUPS: std::sync::LazyLock<CleanupRegistry> =
    std::sync::LazyLock::new(CleanupRegistry::new);

#[cfg(test)]
thread_local! {
    static TEST_DURABLE_CLEANUPS: CleanupRegistry = CleanupRegistry::new();
}

fn wait_for_supervisor_until(
    mut supervisor: CleanupSupervisor,
    context: &'static str,
    registry: &CleanupRegistry,
    deadline: Instant,
) -> io::Result<Option<CleanupOutcome>> {
    loop {
        match supervisor.poll() {
            Ok(()) if supervisor.is_complete() => return supervisor.take_completion(),
            Ok(()) => {}
            Err(error) if supervisor.task.is_complete() => return Err(error),
            Err(error) => {
                let progress = supervisor.progress_detail();
                registry.retain(DurableCleanup::Running(supervisor));
                let progress = if progress.is_empty() {
                    String::new()
                } else {
                    format!("; {progress}")
                };
                return Err(io::Error::new(
                    error.kind(),
                    format!("{error}{progress}; unfinished cleanup remains durably owned"),
                ));
            }
        }
        if Instant::now() >= deadline {
            let progress = supervisor.progress_detail();
            registry.retain(DurableCleanup::Running(supervisor));
            let progress = if progress.is_empty() {
                String::new()
            } else {
                format!("; {progress}")
            };
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{context} exceeded its {PROBE_CLEANUP_GRACE:?} grace period{progress}; \
                     unfinished cleanup remains durably owned"
                ),
            ));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        thread::sleep(remaining.min(Duration::from_millis(5)));
    }
}

fn cancel_cleanup_supervisor(
    mut supervisor: CleanupSupervisor,
    registry: &CleanupRegistry,
    deadline: Instant,
) -> io::Result<()> {
    if let Err(error) = supervisor.cancel() {
        drop(supervisor.take_caller_lease());
        drop(supervisor);
        return Err(error);
    }
    match wait_for_supervisor_until(
        supervisor,
        "probe cleanup supervisor cancellation",
        registry,
        deadline,
    )? {
        None => Ok(()),
        Some(_) => Err(io::Error::other(
            "cancelled cleanup supervisor unexpectedly ran cleanup work",
        )),
    }
}

fn cleanup_probe_group(
    work: CleanupWork,
    supervisor: CleanupSupervisor,
    registry: &CleanupRegistry,
    spawner: &dyn ThreadSpawner,
    deadline: Instant,
) -> io::Result<CleanupOutcome> {
    cleanup_probe_group_with_terminator(work, supervisor, registry, spawner, deadline)
}

fn cleanup_probe_group_with_terminator(
    work: CleanupWork,
    mut supervisor: CleanupSupervisor,
    registry: &CleanupRegistry,
    spawner: &dyn ThreadSpawner,
    deadline: Instant,
) -> io::Result<CleanupOutcome> {
    let work = Box::new(work);
    if let Err(submission) = supervisor.submit(work) {
        let CleanupSubmissionError { error, mut work } = submission;
        let lease = supervisor.take_caller_lease()?;
        drop(supervisor);
        work.record_error(format!("cleanup supervisor submission failed: {error}"));

        supervisor = match CleanupSupervisor::spawn_with(spawner, &lease, registry.workers()) {
            Ok(supervisor) => supervisor,
            Err(restart_error) => {
                let cause = format!("{error}; cleanup supervisor restart failed: {restart_error}");
                let detail = format!("{cause}; cleanup ownership remains durably retained");
                registry.retain(DurableCleanup::Stalled { work, cause });
                drop(lease);
                return Err(io::Error::new(restart_error.kind(), detail));
            }
        };
        drop(lease);
        if let Err(retry) = supervisor.submit(work) {
            let lease = supervisor.take_caller_lease()?;
            drop(supervisor);
            let cause = format!(
                "{error}; replacement cleanup supervisor submission failed: {}",
                retry.error
            );
            let detail = format!("{cause}; cleanup ownership remains durably retained");
            registry.retain(DurableCleanup::Stalled {
                work: retry.work,
                cause,
            });
            drop(lease);
            return Err(io::Error::new(retry.error.kind(), detail));
        }
    }

    match wait_for_supervisor_until(supervisor, "probe cleanup", registry, deadline) {
        Ok(Some(outcome)) => Ok(outcome),
        Ok(None) => Err(io::Error::other(
            "probe cleanup supervisor cancelled without cleanup work",
        )),
        Err(error) => Err(error),
    }
}

fn timed_out_probe(executable: &str, args: &[String], timeout: Duration) -> io::Error {
    let probe = display_invocation(executable, args);
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("probe `{probe}` timed out after {timeout:?}"),
    )
}

fn run_probe_command(
    executable: &str,
    args: &[String],
    timeout: Duration,
) -> io::Result<ProbeOutput> {
    run_probe_command_with_spawner(executable, args, timeout, &SystemThreadSpawner)
}

fn combine_probe_cleanup_error(error: io::Error, cleanup: io::Result<()>) -> io::Error {
    match cleanup {
        Ok(()) => error,
        Err(cleanup_error) => io::Error::new(
            error.kind(),
            format!("{error}; cleanup failed: {cleanup_error}"),
        ),
    }
}

fn cleanup_after_start_error(
    error: io::Error,
    child: ProbeChild,
    stdout_reader: Option<BackgroundTask<io::Result<CapturedStream>>>,
    stderr_reader: Option<BackgroundTask<io::Result<CapturedStream>>>,
    supervisor: CleanupSupervisor,
    registry: &CleanupRegistry,
    spawner: &dyn ThreadSpawner,
) -> io::Error {
    let deadline = Instant::now() + PROBE_CLEANUP_GRACE;
    let cleanup = cleanup_probe_group(
        CleanupWork {
            child,
            stdout_reader,
            stderr_reader,
            leader_exited: false,
            status: None,
            stdout: None,
            stderr: None,
            errors: Vec::new(),
            omitted_errors: 0,
            group_exited: false,
            terminate: Box::new(|child| child.kill()),
            progress: None,
            #[cfg(unix)]
            deferred_group_permission: None,
            permission_denied: false,
        },
        supervisor,
        registry,
        spawner,
        deadline,
    )
    .map(drop);
    combine_probe_cleanup_error(error, cleanup)
}

#[cfg(windows)]
fn cleanup_after_spawn_error(
    error: io::Error,
    child: ProbeChild,
    supervisor: CleanupSupervisor,
    registry: &CleanupRegistry,
    spawner: &dyn ThreadSpawner,
) -> io::Error {
    cleanup_after_start_error(error, child, None, None, supervisor, registry, spawner)
}

fn run_probe_command_with_spawner(
    executable: &str,
    args: &[String],
    timeout: Duration,
    spawner: &dyn ThreadSpawner,
) -> io::Result<ProbeOutput> {
    run_probe_command_with_spawner_and_terminator(executable, args, timeout, spawner, |child| {
        child.kill()
    })
}

fn run_probe_command_with_spawner_and_terminator(
    executable: &str,
    args: &[String],
    timeout: Duration,
    spawner: &dyn ThreadSpawner,
    terminate: impl FnMut(&mut ProbeChild) -> io::Result<()> + Send + 'static,
) -> io::Result<ProbeOutput> {
    #[cfg(test)]
    {
        TEST_DURABLE_CLEANUPS.with(|registry| {
            run_probe_command_with_registry(executable, args, timeout, spawner, registry, terminate)
        })
    }
    #[cfg(not(test))]
    {
        run_probe_command_with_registry(
            executable,
            args,
            timeout,
            spawner,
            &DURABLE_CLEANUPS,
            terminate,
        )
    }
}

fn run_probe_command_with_registry(
    executable: &str,
    args: &[String],
    timeout: Duration,
    spawner: &dyn ThreadSpawner,
    registry: &CleanupRegistry,
    terminate: impl FnMut(&mut ProbeChild) -> io::Result<()> + Send + 'static,
) -> io::Result<ProbeOutput> {
    let lease = registry.acquire()?;
    registry.preflight(&lease, spawner)?;
    let supervisor =
        CleanupSupervisor::spawn_with(spawner, &lease, registry.workers()).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to spawn probe cleanup supervisor: {error}"),
            )
        })?;
    drop(lease);
    let deadline = Instant::now() + timeout;
    let mut command = Command::new(executable);
    command
        .args(args)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match ProbeChild::spawn(&mut command) {
        Ok(child) => child,
        Err(SpawnProbeError::BeforeSpawn(error)) => {
            let cleanup_deadline = Instant::now() + PROBE_CLEANUP_GRACE;
            return Err(combine_probe_cleanup_error(
                error,
                cancel_cleanup_supervisor(supervisor, registry, cleanup_deadline),
            ));
        }
        #[cfg(windows)]
        Err(SpawnProbeError::PostSpawn { error, child }) => {
            return Err(cleanup_after_spawn_error(
                error, *child, supervisor, registry, spawner,
            ));
        }
    };
    let stdout = match child.take_stdout() {
        Some(stdout) => stdout,
        None => {
            return Err(cleanup_after_start_error(
                io::Error::other("piped probe stdout was unavailable after spawn"),
                child,
                None,
                None,
                supervisor,
                registry,
                spawner,
            ));
        }
    };
    let stderr = match child.take_stderr() {
        Some(stderr) => stderr,
        None => {
            return Err(cleanup_after_start_error(
                io::Error::other("piped probe stderr was unavailable after spawn"),
                child,
                None,
                None,
                supervisor,
                registry,
                spawner,
            ));
        }
    };
    let mut stdout_reader =
        match capture_stream_async(stdout, "stdout reader", spawner, registry.workers()) {
            Ok(reader) => reader,
            Err(error) => {
                return Err(cleanup_after_start_error(
                    io::Error::new(
                        error.kind(),
                        format!("failed to spawn stdout reader: {error}"),
                    ),
                    child,
                    None,
                    None,
                    supervisor,
                    registry,
                    spawner,
                ));
            }
        };
    let mut stderr_reader =
        match capture_stream_async(stderr, "stderr reader", spawner, registry.workers()) {
            Ok(reader) => reader,
            Err(error) => {
                return Err(cleanup_after_start_error(
                    io::Error::new(
                        error.kind(),
                        format!("failed to spawn stderr reader: {error}"),
                    ),
                    child,
                    Some(stdout_reader),
                    None,
                    supervisor,
                    registry,
                    spawner,
                ));
            }
        };
    let mut leader_exited = false;
    let mut stdout = None;
    let mut stderr = None;

    let probe_result = loop {
        if let Err(error) = poll_capture(&mut stdout_reader, &mut stdout) {
            break Err(error);
        }
        if let Err(error) = poll_capture(&mut stderr_reader, &mut stderr) {
            break Err(error);
        }

        if !leader_exited {
            match child.try_wait_leader() {
                Ok(exited) => leader_exited = exited,
                Err(error) => break Err(error),
            }
        }

        if leader_exited && stdout.is_some() && stderr.is_some() {
            break Ok(());
        }

        if Instant::now() >= deadline {
            break Err(timed_out_probe(executable, args, timeout));
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        thread::sleep(remaining.min(Duration::from_millis(10)));
    };

    let cleanup_deadline = Instant::now() + PROBE_CLEANUP_GRACE;
    let cleanup_result = cleanup_probe_group_with_terminator(
        CleanupWork {
            child,
            stdout_reader: Some(stdout_reader),
            stderr_reader: Some(stderr_reader),
            leader_exited,
            status: None,
            stdout,
            stderr,
            errors: Vec::new(),
            omitted_errors: 0,
            group_exited: false,
            terminate: Box::new(terminate),
            progress: None,
            #[cfg(unix)]
            deferred_group_permission: None,
            permission_denied: false,
        },
        supervisor,
        registry,
        spawner,
        cleanup_deadline,
    );
    match (probe_result, cleanup_result) {
        (Ok(()), Ok(cleanup)) => {
            let stdout = cleanup
                .stdout
                .ok_or_else(|| io::Error::other("probe stdout capture did not complete"))?;
            let stderr = cleanup
                .stderr
                .ok_or_else(|| io::Error::other("probe stderr capture did not complete"))?;
            Ok(ProbeOutput {
                status: cleanup.status,
                stdout,
                stderr,
            })
        }
        (Err(error), Ok(_)) => Err(error),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(error), Err(cleanup_error)) => Err(io::Error::new(
            error.kind(),
            format!("{error}; cleanup failed: {cleanup_error}"),
        )),
    }
}

fn failed_probe_diagnostic(probe: &str, output: &ProbeOutput) -> String {
    format!(
        "probe `{probe}` exited with {}; {}; {}",
        output.status,
        diagnostic_context("stdout", &output.stdout),
        diagnostic_context("stderr", &output.stderr)
    )
}

fn diagnostic_context(label: &str, stream: &CapturedStream) -> String {
    let compact = normalized_whitespace(&stream.text);
    if compact.is_empty() {
        return format!("{label}: <empty>");
    }
    let mut chars = compact.chars();
    let excerpt: String = chars.by_ref().take(MAX_DIAGNOSTIC_CHARS).collect();
    if stream.truncated || chars.next().is_some() {
        format!("{label}: {excerpt} … [truncated]")
    } else {
        format!("{label}: {excerpt}")
    }
}

/// Soft checks: if the adapter declares a flag, note when the successful probe
/// output doesn't mention it. Covers every flag the manifest can declare —
/// model, system-prompt, prompt binding, sandbox, stream, continuity, and the
/// long-form tokens of every launch-arg list — since a typo in any of them only
/// surfaces at real session time otherwise. Never fails — CLIs don't always
/// list every flag in subcommand help, and root `--version` output is short.
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
        let flag = flag.trim();
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
    #[cfg(unix)]
    use super::probe_child::UnixGroupApi;
    use super::*;
    use coven_runtime_spec::{Capabilities, ModelIdTransform, SandboxMapping};
    #[cfg(unix)]
    use std::sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Barrier,
    };

    #[cfg(unix)]
    static PROBE_TIMING_TESTS: Mutex<()> = Mutex::new(());

    #[cfg(unix)]
    fn serialize_probe_timing_test() -> std::sync::MutexGuard<'static, ()> {
        PROBE_TIMING_TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(unix)]
    struct FailNthThreadSpawner {
        fail_at: usize,
        calls: AtomicUsize,
        failure_delay: Duration,
    }

    #[cfg(unix)]
    impl FailNthThreadSpawner {
        fn new(fail_at: usize, failure_delay: Duration) -> Self {
            Self {
                fail_at,
                calls: AtomicUsize::new(0),
                failure_delay,
            }
        }
    }

    #[cfg(unix)]
    impl ThreadSpawner for FailNthThreadSpawner {
        fn spawn(
            &self,
            name: String,
            operation: Box<dyn FnOnce() + Send + 'static>,
        ) -> io::Result<JoinHandle<()>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == self.fail_at {
                thread::sleep(self.failure_delay);
                return Err(io::Error::other(format!(
                    "injected worker creation failure #{call}"
                )));
            }
            thread::Builder::new().name(name).spawn(operation)
        }
    }

    #[cfg(unix)]
    struct CountingThreadSpawner {
        calls: AtomicUsize,
    }

    #[cfg(unix)]
    impl CountingThreadSpawner {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[cfg(unix)]
    impl ThreadSpawner for CountingThreadSpawner {
        fn spawn(
            &self,
            name: String,
            operation: Box<dyn FnOnce() + Send + 'static>,
        ) -> io::Result<JoinHandle<()>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            thread::Builder::new().name(name).spawn(operation)
        }
    }

    #[cfg(unix)]
    struct FirstSpawnBarrierSpawner {
        calls: AtomicUsize,
        first_entered: Arc<Barrier>,
        release_first: Arc<Barrier>,
    }

    #[cfg(unix)]
    impl FirstSpawnBarrierSpawner {
        fn new(first_entered: Arc<Barrier>, release_first: Arc<Barrier>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                first_entered,
                release_first,
            }
        }
    }

    #[cfg(unix)]
    impl ThreadSpawner for FirstSpawnBarrierSpawner {
        fn spawn(
            &self,
            name: String,
            operation: Box<dyn FnOnce() + Send + 'static>,
        ) -> io::Result<JoinHandle<()>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == 1 {
                self.first_entered.wait();
                self.release_first.wait();
            }
            thread::Builder::new().name(name).spawn(operation)
        }
    }

    #[cfg(unix)]
    struct DisconnectFirstThreadSpawner {
        calls: AtomicUsize,
        disconnect_delay: Duration,
    }

    #[cfg(unix)]
    impl DisconnectFirstThreadSpawner {
        fn new(disconnect_delay: Duration) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                disconnect_delay,
            }
        }
    }

    #[cfg(unix)]
    impl ThreadSpawner for DisconnectFirstThreadSpawner {
        fn spawn(
            &self,
            name: String,
            operation: Box<dyn FnOnce() + Send + 'static>,
        ) -> io::Result<JoinHandle<()>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call != 1 {
                return thread::Builder::new().name(name).spawn(operation);
            }

            let (disconnected, ready) = mpsc::sync_channel(0);
            let delay = self.disconnect_delay;
            let handle = thread::Builder::new().name(name).spawn(move || {
                drop(operation);
                let _ = disconnected.send(());
                thread::sleep(delay);
            })?;
            ready.recv().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "injected supervisor did not disconnect",
                )
            })?;
            Ok(handle)
        }
    }

    #[cfg(unix)]
    struct StalledRecoverySpawner {
        calls: AtomicUsize,
    }

    #[cfg(unix)]
    impl StalledRecoverySpawner {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[cfg(unix)]
    impl ThreadSpawner for StalledRecoverySpawner {
        fn spawn(
            &self,
            name: String,
            operation: Box<dyn FnOnce() + Send + 'static>,
        ) -> io::Result<JoinHandle<()>> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            match call {
                // Disconnect the original supervisor before submission.
                1 => thread::Builder::new()
                    .name(name)
                    .spawn(drop_operation(operation)),
                // Calls 2-3 are the pipe readers. Fail the disconnected
                // supervisor's immediate replacement and the first preflight
                // restart. The following preflight must recover.
                4 | 5 => Err(io::Error::other(format!(
                    "injected cleanup supervisor spawn failure #{call}"
                ))),
                _ => thread::Builder::new().name(name).spawn(operation),
            }
        }
    }

    #[cfg(unix)]
    fn drop_operation(operation: Box<dyn FnOnce() + Send + 'static>) -> impl FnOnce() + Send {
        move || drop(operation)
    }

    #[cfg(unix)]
    fn write_stop_file(dir: &tempfile::TempDir) {
        std::fs::write(dir.path().join("stop"), b"stop").expect("write controlled-child stop file");
    }

    #[cfg(unix)]
    fn controlled_endless_script(name: &str) -> (tempfile::TempDir, PathBuf) {
        unix_script(
            name,
            r#"#!/bin/sh
marker_dir="${0%/*}"
printf started >> "$marker_dir/spawned"
while [ ! -f "$marker_dir/stop" ]; do
  sleep 0.02
done
printf natural > "$marker_dir/natural-exit"
"#,
        )
    }

    #[cfg(unix)]
    fn wait_for_attempts(attempts: &AtomicUsize, minimum: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if attempts.load(Ordering::SeqCst) >= minimum {
                return true;
            }
            thread::sleep(Duration::from_millis(10));
        }
        attempts.load(Ordering::SeqCst) >= minimum
    }

    #[cfg(unix)]
    fn wait_for_worker_count(workers: &WorkerSet, expected: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if workers
                .retained
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
                == expected
            {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        workers
            .retained
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
            == expected
    }

    #[cfg(unix)]
    #[derive(Clone, Copy)]
    enum InjectedGroupResult {
        Exists,
        Gone,
        PermissionDenied,
    }

    #[cfg(unix)]
    struct FaultUnixGroupApi {
        group_results: Mutex<std::collections::VecDeque<InjectedGroupResult>>,
        kill_permission_denied: bool,
        group_calls: AtomicUsize,
        kill_calls: AtomicUsize,
    }

    #[cfg(unix)]
    impl FaultUnixGroupApi {
        fn new(
            group_results: impl IntoIterator<Item = InjectedGroupResult>,
            kill_permission_denied: bool,
        ) -> Self {
            Self {
                group_results: Mutex::new(group_results.into_iter().collect()),
                kill_permission_denied,
                group_calls: AtomicUsize::new(0),
                kill_calls: AtomicUsize::new(0),
            }
        }
    }

    #[cfg(unix)]
    impl UnixGroupApi for FaultUnixGroupApi {
        fn group_exists(&self, _process_group: i32) -> io::Result<bool> {
            self.group_calls.fetch_add(1, Ordering::SeqCst);
            match self
                .group_results
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front()
                .unwrap_or(InjectedGroupResult::Gone)
            {
                InjectedGroupResult::Exists => Ok(true),
                InjectedGroupResult::Gone => Ok(false),
                InjectedGroupResult::PermissionDenied => {
                    Err(io::Error::from_raw_os_error(libc::EPERM))
                }
            }
        }

        fn kill_group(&self, child: &mut command_group::GroupChild) -> io::Result<()> {
            self.kill_calls.fetch_add(1, Ordering::SeqCst);
            if self.kill_permission_denied {
                Err(io::Error::from_raw_os_error(libc::EPERM))
            } else {
                child.kill()
            }
        }
    }

    #[cfg(unix)]
    fn faulted_probe_child(
        name: &str,
        body: &str,
        api: Arc<dyn UnixGroupApi>,
    ) -> (tempfile::TempDir, ProbeChild) {
        let (dir, executable) = unix_script(name, body);
        let mut command = Command::new(executable);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = match ProbeChild::spawn_with_group_api(&mut command, api) {
            Ok(child) => child,
            Err(SpawnProbeError::BeforeSpawn(error)) => {
                panic!("spawn fault-injected grouped child: {error}")
            }
            #[cfg(windows)]
            Err(SpawnProbeError::PostSpawn { error, .. }) => {
                panic!("prepare fault-injected grouped child: {error}")
            }
        };
        (dir, child)
    }

    #[cfg(unix)]
    fn cleanup_work_without_readers(child: ProbeChild) -> CleanupWork {
        CleanupWork {
            child,
            stdout_reader: None,
            stderr_reader: None,
            leader_exited: false,
            status: None,
            stdout: None,
            stderr: None,
            errors: Vec::new(),
            omitted_errors: 0,
            group_exited: false,
            terminate: Box::new(ProbeChild::kill),
            progress: None,
            deferred_group_permission: None,
            permission_denied: false,
        }
    }

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
            model_id_transform: ModelIdTransform::StripProvider,
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

    #[cfg(unix)]
    #[test]
    fn supervisor_creation_failure_is_returned_before_child_spawn() {
        let (dir, executable) = unix_script(
            "must-not-start",
            "#!/bin/sh\n: > \"${0%/*}/child-started\"\nsleep 2\n",
        );
        let spawner = FailNthThreadSpawner::new(1, Duration::ZERO);
        let args = ["--help".into()];

        let result = run_probe_command_with_spawner(
            executable.to_str().unwrap(),
            &args,
            Duration::from_secs(1),
            &spawner,
        );

        let error = match result {
            Err(error) => error,
            Ok(output) => panic!("injected spawn failure returned {}", output.status),
        };
        assert!(
            error
                .to_string()
                .contains("injected worker creation failure #1"),
            "{error}"
        );
        assert!(
            !dir.path().join("child-started").exists(),
            "child spawned before the cleanup supervisor was fallibly created"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reader_creation_failures_after_spawn_clean_up_and_are_returned() {
        let _timing = serialize_probe_timing_test();
        for (fail_at, label) in [(2, "stdout"), (3, "stderr")] {
            let name = format!("{label}-reader-failure");
            let (dir, executable) = unix_script(
                &name,
                "#!/bin/sh\nsleep 1\n: > \"${0%/*}/child-survived\"\n",
            );
            let spawner = FailNthThreadSpawner::new(fail_at, Duration::from_millis(100));
            let args = ["--help".into()];
            let started = Instant::now();

            let result = run_probe_command_with_spawner(
                executable.to_str().unwrap(),
                &args,
                Duration::from_secs(1),
                &spawner,
            );
            let elapsed = started.elapsed();

            let error = match result {
                Err(error) => error,
                Ok(output) => panic!("injected {label} spawn failure returned {}", output.status),
            };
            assert!(
                error
                    .to_string()
                    .contains(&format!("injected worker creation failure #{fail_at}")),
                "{error}"
            );
            assert_eq!(
                spawner.calls.load(Ordering::SeqCst),
                fail_at,
                "the injected failure must occur at {label}-reader creation"
            );
            assert!(
                elapsed < Duration::from_secs(1),
                "post-spawn {label} worker failure cleanup exceeded its bound: {elapsed:?}"
            );
            thread::sleep(Duration::from_millis(1_100));
            assert!(
                !dir.path().join("child-survived").exists(),
                "post-spawn {label} worker failure left the child running"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn kill_failure_is_surfaced_and_cleanup_stays_durably_owned() {
        let _timing = serialize_probe_timing_test();
        let registry = CleanupRegistry::new();
        let (_dir, executable) = unix_script("kill-failure", "#!/bin/sh\nsleep 3\n");
        let args = ["--help".into()];

        let result = run_probe_command_with_registry(
            executable.to_str().unwrap(),
            &args,
            Duration::from_secs(1),
            &SystemThreadSpawner,
            &registry,
            |_child| {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected process-group kill failure",
                ))
            },
        );

        let error = match result {
            Err(error) => error,
            Ok(output) => panic!("injected kill failure returned {}", output.status),
        };
        assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
        assert!(
            error
                .to_string()
                .contains("injected process-group kill failure"),
            "{error}"
        );
        assert!(error.to_string().contains("cleanup exceeded"), "{error}");

        let (retry_dir, retry) = unix_script(
            "kill-failure-retry",
            "#!/bin/sh\n: > \"${0%/*}/retry-started\"\necho 'Usage: retry [OPTIONS]'\n",
        );
        let immediate = run_probe_command_with_registry(
            retry.to_str().unwrap(),
            &args,
            Duration::from_secs(5),
            &SystemThreadSpawner,
            &registry,
            ProbeChild::kill,
        );
        let immediate_error = match immediate {
            Err(error) => error,
            Ok(output) => panic!("retry during kill cleanup returned {}", output.status),
        };
        assert_eq!(
            immediate_error.kind(),
            io::ErrorKind::WouldBlock,
            "{immediate_error}"
        );
        assert!(!retry_dir.path().join("retry-started").exists());

        let surfaced_deadline = Instant::now() + Duration::from_secs(5);
        let surfaced = loop {
            let result = run_probe_command_with_registry(
                retry.to_str().unwrap(),
                &args,
                Duration::from_secs(5),
                &SystemThreadSpawner,
                &registry,
                ProbeChild::kill,
            );
            if result
                .as_ref()
                .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock)
                && Instant::now() < surfaced_deadline
            {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            break result;
        };
        let surfaced_error = match surfaced {
            Err(error) => error,
            Ok(output) => panic!(
                "deferred kill error was lost; retry returned {}",
                output.status
            ),
        };
        assert!(
            surfaced_error
                .to_string()
                .contains("injected process-group kill failure"),
            "{surfaced_error}"
        );
        assert!(!retry_dir.path().join("retry-started").exists());

        let recovered = run_probe_command_with_registry(
            retry.to_str().unwrap(),
            &args,
            Duration::from_secs(5),
            &SystemThreadSpawner,
            &registry,
            ProbeChild::kill,
        )
        .expect("completed cleanup must permit a retry after surfacing its error");
        assert!(recovered.status.success(), "{}", recovered.status);
        assert!(retry_dir.path().join("retry-started").exists());
    }

    #[cfg(unix)]
    #[test]
    fn transient_kill_failure_is_retried_without_a_future_probe() {
        let _timing = serialize_probe_timing_test();
        let registry = CleanupRegistry::new();
        let (dir, executable) = controlled_endless_script("transient-kill-failure");
        let args = ["--help".into()];
        let attempts = Arc::new(AtomicUsize::new(0));
        let cleanup_attempts = Arc::clone(&attempts);
        let started = Instant::now();

        let result = run_probe_command_with_registry(
            executable.to_str().unwrap(),
            &args,
            Duration::from_millis(50),
            &SystemThreadSpawner,
            &registry,
            move |child| {
                let attempt = cleanup_attempts.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected transient process-group kill failure",
                    ))
                } else {
                    child.kill()
                }
            },
        );
        let elapsed = started.elapsed();
        let retried_before_stop = wait_for_attempts(&attempts, 2, Duration::from_millis(750));

        // Always release the committed implementation's endless fixture before
        // asserting the RED expectations.
        write_stop_file(&dir);
        thread::sleep(Duration::from_millis(250));

        let error = match result {
            Err(error) => error,
            Ok(output) => panic!("timed-out controlled child returned {}", output.status),
        };
        assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
        assert!(
            elapsed < Duration::from_millis(800),
            "transient kill cleanup exceeded its caller bound: {elapsed:?}"
        );
        assert!(
            retried_before_stop,
            "termination was attempted only {} time(s) before an external stop",
            attempts.load(Ordering::SeqCst)
        );
        assert!(
            !dir.path().join("natural-exit").exists(),
            "the endless child exited only because the test released it"
        );
    }

    #[cfg(unix)]
    #[test]
    fn blocking_permanent_terminator_is_bounded_and_retains_one_probe() {
        let _timing = serialize_probe_timing_test();
        let registry = CleanupRegistry::new();
        let spawner = CountingThreadSpawner::new();
        let (dir, executable) = controlled_endless_script("blocking-terminator");
        let args = ["--help".into()];
        let release_block = Arc::new(AtomicBool::new(false));
        let allow_kill = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));
        let release_for_thread = Arc::clone(&release_block);
        let releaser = thread::spawn(move || {
            thread::sleep(Duration::from_millis(1_100));
            release_for_thread.store(true, Ordering::SeqCst);
        });
        let release_for_cleanup = Arc::clone(&release_block);
        let allow_kill_for_cleanup = Arc::clone(&allow_kill);
        let attempts_for_cleanup = Arc::clone(&attempts);
        let started = Instant::now();

        let first = run_probe_command_with_registry(
            executable.to_str().unwrap(),
            &args,
            Duration::from_millis(200),
            &spawner,
            &registry,
            move |child| {
                attempts_for_cleanup.fetch_add(1, Ordering::SeqCst);
                while !release_for_cleanup.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(10));
                }
                if allow_kill_for_cleanup.load(Ordering::SeqCst) {
                    child.kill()
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected permanent process-group kill failure",
                    ))
                }
            },
        );
        let first_elapsed = started.elapsed();

        let (retry_dir, retry) = unix_script(
            "blocking-terminator-retry",
            "#!/bin/sh\n: > \"${0%/*}/retry-started\"\necho 'Usage: retry [OPTIONS]'\n",
        );
        let mut retries = Vec::new();
        for _ in 0..3 {
            let retry_started = Instant::now();
            let result = run_probe_command_with_registry(
                retry.to_str().unwrap(),
                &args,
                Duration::from_secs(1),
                &spawner,
                &registry,
                ProbeChild::kill,
            );
            retries.push((result, retry_started.elapsed()));
        }
        let worker_calls_while_retained = spawner.calls.load(Ordering::SeqCst);

        releaser.join().expect("release blocking terminator");
        let automatically_retried = wait_for_attempts(&attempts, 2, Duration::from_millis(250));
        allow_kill.store(true, Ordering::SeqCst);
        write_stop_file(&dir);
        thread::sleep(Duration::from_millis(250));

        let first_error = match first {
            Err(error) => error,
            Ok(output) => panic!("timed-out controlled child returned {}", output.status),
        };
        assert_eq!(first_error.kind(), io::ErrorKind::TimedOut, "{first_error}");
        for (retry_result, retry_elapsed) in retries {
            let retry_error = match retry_result {
                Err(error) => error,
                Ok(output) => panic!("retained cleanup retry returned {}", output.status),
            };
            assert_eq!(
                retry_error.kind(),
                io::ErrorKind::WouldBlock,
                "{retry_error}"
            );
            assert!(
                retry_elapsed < Duration::from_millis(250),
                "retained-cleanup admission blocked its caller: {retry_elapsed:?}"
            );
        }
        assert!(
            first_elapsed < Duration::from_millis(900),
            "blocking termination escaped the cleanup deadline: {first_elapsed:?}"
        );
        assert!(
            automatically_retried,
            "permanent termination was attempted only {} time(s)",
            attempts.load(Ordering::SeqCst)
        );
        assert_eq!(
            worker_calls_while_retained, 3,
            "a rejected retry created an additional cleanup or reader worker"
        );
        assert!(
            !retry_dir.path().join("retry-started").exists(),
            "a retained-cleanup retry spawned another child"
        );
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_admission_allows_exactly_one_probe() {
        let _timing = serialize_probe_timing_test();
        let registry = Arc::new(CleanupRegistry::new());
        let first_entered = Arc::new(Barrier::new(2));
        let release_first = Arc::new(Barrier::new(2));
        let spawner = Arc::new(FirstSpawnBarrierSpawner::new(
            Arc::clone(&first_entered),
            Arc::clone(&release_first),
        ));
        let (dir, executable) = unix_script(
            "concurrent-admission",
            "#!/bin/sh\nprintf 'started\\n' >> \"${0%/*}/spawned\"\necho 'Usage: probe [OPTIONS]'\n",
        );

        let first_registry = Arc::clone(&registry);
        let first_spawner = Arc::clone(&spawner);
        let first_executable = executable.clone();
        let first = thread::spawn(move || {
            let started = Instant::now();
            let result = run_probe_command_with_registry(
                first_executable.to_str().unwrap(),
                &["--help".into()],
                Duration::from_secs(5),
                first_spawner.as_ref(),
                first_registry.as_ref(),
                ProbeChild::kill,
            );
            (result, started.elapsed())
        });

        first_entered.wait();
        let second_registry = Arc::clone(&registry);
        let second_spawner = Arc::clone(&spawner);
        let second_executable = executable;
        let second = thread::spawn(move || {
            let started = Instant::now();
            let result = run_probe_command_with_registry(
                second_executable.to_str().unwrap(),
                &["--help".into()],
                Duration::from_secs(5),
                second_spawner.as_ref(),
                second_registry.as_ref(),
                ProbeChild::kill,
            );
            (result, started.elapsed())
        });

        // The first contender is held inside supervisor creation with its
        // admission lease active. Joining the second before release proves it
        // was actually scheduled and rejected, rather than relying on a short
        // scheduler-timing window.
        let second_outcome = second.join().expect("second concurrent probe");
        let second_reached_worker_creation = spawner.calls.load(Ordering::SeqCst) >= 2;
        release_first.wait();
        let outcomes = [
            first.join().expect("first concurrent probe"),
            second_outcome,
        ];

        let successes = outcomes.iter().filter(|(result, _)| result.is_ok()).count();
        let rejected = outcomes
            .iter()
            .filter(|(result, _)| {
                result
                    .as_ref()
                    .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock)
            })
            .count();
        let rejected_elapsed = outcomes.iter().find_map(|(result, elapsed)| {
            result
                .as_ref()
                .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock)
                .then_some(*elapsed)
        });
        let spawn_count = std::fs::read_to_string(dir.path().join("spawned"))
            .expect("read concurrent spawn marker")
            .lines()
            .count();

        assert!(
            !second_reached_worker_creation,
            "both contenders passed admission and created cleanup supervisors"
        );
        assert_eq!(
            successes, 1,
            "concurrent success/rejection counts: {successes}/{rejected}"
        );
        assert_eq!(
            rejected, 1,
            "concurrent success/rejection counts: {successes}/{rejected}"
        );
        assert!(
            rejected_elapsed.is_some_and(|elapsed| elapsed < Duration::from_millis(250)),
            "concurrent rejection was not immediate: {rejected_elapsed:?}"
        );
        assert_eq!(spawn_count, 1, "both concurrent probes spawned children");
    }

    #[cfg(unix)]
    #[test]
    fn retained_cleanup_is_scoped_to_its_registry() {
        let _timing = serialize_probe_timing_test();
        let registry_a = CleanupRegistry::new();
        let registry_b = CleanupRegistry::new();
        let disconnected_spawner = DisconnectFirstThreadSpawner::new(Duration::from_millis(500));
        let (_first_dir, first_executable) = unix_script(
            "registry-a-disconnected-supervisor",
            "#!/bin/sh\necho 'Usage: first [OPTIONS]'\n",
        );
        let (second_dir, second_executable) = unix_script(
            "registry-b-isolated",
            "#!/bin/sh\n: > \"${0%/*}/second-started\"\necho 'Usage: second [OPTIONS]'\n",
        );
        let args = ["--help".into()];

        let first = run_probe_command_with_registry(
            first_executable.to_str().unwrap(),
            &args,
            Duration::from_secs(1),
            &disconnected_spawner,
            &registry_a,
            ProbeChild::kill,
        );
        let second = run_probe_command_with_registry(
            second_executable.to_str().unwrap(),
            &args,
            Duration::from_secs(1),
            &SystemThreadSpawner,
            &registry_b,
            ProbeChild::kill,
        );

        // Let the injected disconnected supervisor finish so every ownership
        // path is safe to release even when the RED assertions fail.
        thread::sleep(Duration::from_millis(650));

        assert!(
            first.is_err(),
            "injected supervisor disconnect unexpectedly passed"
        );
        let output = second.expect("registry A's retained worker poisoned registry B");
        assert!(output.status.success(), "{}", output.status);
        assert!(second_dir.path().join("second-started").exists());
    }

    #[cfg(unix)]
    #[test]
    fn stalled_cleanup_retries_twice_then_recovers_and_surfaces_errors() {
        let _timing = serialize_probe_timing_test();
        let registry = CleanupRegistry::new();
        let spawner = StalledRecoverySpawner::new();
        let (first_dir, first_executable) = controlled_endless_script("stalled-recovery");
        let (retry_dir, retry_executable) = unix_script(
            "stalled-recovery-retry",
            "#!/bin/sh\n: > \"${0%/*}/retry-started\"\necho 'Usage: retry [OPTIONS]'\n",
        );
        let args = ["--help".into()];

        let first = run_probe_command_with_registry(
            first_executable.to_str().unwrap(),
            &args,
            Duration::from_millis(50),
            &spawner,
            &registry,
            ProbeChild::kill,
        );
        let first_error = match first {
            Err(error) => error,
            Ok(output) => panic!("injected replacement failure returned {}", output.status),
        };
        assert!(
            first_error
                .to_string()
                .contains("injected cleanup supervisor spawn failure #4"),
            "{first_error}"
        );

        let second = run_probe_command_with_registry(
            retry_executable.to_str().unwrap(),
            &args,
            Duration::from_secs(1),
            &spawner,
            &registry,
            ProbeChild::kill,
        );
        let second_error = match second {
            Err(error) => error,
            Ok(output) => panic!("first preflight restart returned {}", output.status),
        };
        assert!(
            second_error
                .to_string()
                .contains("injected cleanup supervisor spawn failure #5"),
            "{second_error}"
        );
        assert_eq!(spawner.calls.load(Ordering::SeqCst), 5);
        assert!(!retry_dir.path().join("retry-started").exists());

        let third = run_probe_command_with_registry(
            retry_executable.to_str().unwrap(),
            &args,
            Duration::from_secs(1),
            &spawner,
            &registry,
            ProbeChild::kill,
        );
        let third_error = match third {
            Err(error) => error,
            Ok(output) => panic!("recovery preflight returned {}", output.status),
        };
        assert_eq!(
            third_error.kind(),
            io::ErrorKind::WouldBlock,
            "{third_error}"
        );
        assert_eq!(spawner.calls.load(Ordering::SeqCst), 6);
        assert!(!retry_dir.path().join("retry-started").exists());

        let surfaced_deadline = Instant::now() + Duration::from_secs(2);
        let surfaced_error = loop {
            let result = run_probe_command_with_registry(
                retry_executable.to_str().unwrap(),
                &args,
                Duration::from_secs(1),
                &spawner,
                &registry,
                ProbeChild::kill,
            );
            match result {
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        && Instant::now() < surfaced_deadline =>
                {
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => break error,
                Ok(output) => panic!(
                    "retained cleanup errors were lost; retry returned {}",
                    output.status
                ),
            }
        };
        assert!(
            surfaced_error
                .to_string()
                .contains("cleanup supervisor submission failed"),
            "{surfaced_error}"
        );
        assert!(
            surfaced_error
                .to_string()
                .contains("injected cleanup supervisor spawn failure #4"),
            "{surfaced_error}"
        );
        assert!(!retry_dir.path().join("retry-started").exists());

        let recovered = run_probe_command_with_registry(
            retry_executable.to_str().unwrap(),
            &args,
            Duration::from_secs(1),
            &spawner,
            &registry,
            ProbeChild::kill,
        )
        .expect("surfaced cleanup failure must release admission");
        assert!(recovered.status.success(), "{}", recovered.status);
        assert!(retry_dir.path().join("retry-started").exists());

        // The recovered cleanup, not fixture release, must have terminated the
        // originally retained process.
        write_stop_file(&first_dir);
        thread::sleep(Duration::from_millis(100));
        assert!(
            !first_dir.path().join("natural-exit").exists(),
            "stalled cleanup only finished after external fixture release"
        );
    }

    #[cfg(unix)]
    #[test]
    fn dropping_worker_set_transfers_unfinished_worker_without_blocking() {
        let workers = Arc::new(WorkerSet::default());
        let release = Arc::new(AtomicBool::new(false));
        let finished = Arc::new(AtomicBool::new(false));
        let release_worker = Arc::clone(&release);
        let finished_worker = Arc::clone(&finished);
        let worker = thread::Builder::new()
            .name("worker-set-drop-fixture".into())
            .spawn(move || {
                while !release_worker.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(5));
                }
                finished_worker.store(true, Ordering::SeqCst);
            })
            .expect("spawn non-returning worker fixture");
        workers.retain(RetainedWorker {
            label: "worker-set-drop-fixture",
            handle: worker,
        });
        assert!(wait_for_worker_count(
            &workers,
            1,
            Duration::from_millis(100)
        ));

        let started = Instant::now();
        drop(workers);
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(250),
            "WorkerSet::drop blocked on an unfinished worker for {elapsed:?}"
        );
        assert!(
            process_lifetime_worker_count("worker-set-drop-fixture") >= 1,
            "unfinished worker handle was detached instead of transferred"
        );

        release.store(true, Ordering::SeqCst);
        let deadline = Instant::now() + Duration::from_secs(1);
        while !finished.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            finished.load(Ordering::SeqCst),
            "worker fixture did not exit"
        );
        reap_process_lifetime_workers();
        assert_eq!(
            process_lifetime_worker_count("worker-set-drop-fixture"),
            0,
            "finished transferred worker was not reaped"
        );
    }

    #[cfg(unix)]
    #[test]
    fn presence_eperm_returns_permission_denied_and_never_reuses_numeric_target() {
        let _timing = serialize_probe_timing_test();
        let api = Arc::new(FaultUnixGroupApi::new(
            [InjectedGroupResult::PermissionDenied],
            false,
        ));
        let (_dir, mut child) = faulted_probe_child(
            "presence-eperm-seam",
            "#!/bin/sh\nsleep 0.05\n",
            Arc::clone(&api) as Arc<dyn UnixGroupApi>,
        );

        let error = child
            .try_wait_group()
            .expect_err("presence EPERM must remain an error");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert!(
            child.try_wait_group().expect("consumed group target"),
            "permission-denied group target remained reusable"
        );
        assert_eq!(api.group_calls.load(Ordering::SeqCst), 1);
    }

    #[cfg(unix)]
    #[test]
    fn kill_eperm_returns_permission_denied_and_never_resignals_numeric_target() {
        let _timing = serialize_probe_timing_test();
        let api = Arc::new(FaultUnixGroupApi::new([InjectedGroupResult::Exists], true));
        let (_dir, mut child) = faulted_probe_child(
            "kill-eperm-seam",
            "#!/bin/sh\nsleep 0.05\n",
            Arc::clone(&api) as Arc<dyn UnixGroupApi>,
        );

        assert!(
            !child.try_wait_group().expect("existing process group"),
            "injected group unexpectedly absent"
        );
        let error = child
            .kill()
            .expect_err("terminal-kill EPERM must remain an error");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        child.kill().expect("consumed group target is a no-op");
        assert_eq!(api.kill_calls.load(Ordering::SeqCst), 1);
    }

    #[cfg(unix)]
    #[test]
    fn group_presence_eperm_consumes_target_and_is_surfaced_after_survivor_check() {
        let _timing = serialize_probe_timing_test();
        let api = Arc::new(FaultUnixGroupApi::new(
            [
                InjectedGroupResult::PermissionDenied,
                InjectedGroupResult::Exists,
            ],
            false,
        ));
        let (_dir, child) = faulted_probe_child(
            "presence-eperm",
            "#!/bin/sh\nexit 0\n",
            Arc::clone(&api) as Arc<dyn UnixGroupApi>,
        );

        let error = match finish_cleanup_work(cleanup_work_without_readers(child)) {
            Err(error) => error,
            Ok(_) => panic!("post-reap survivor suppressed deferred presence EPERM"),
        };

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert!(
            error
                .to_string()
                .contains("permission denied while polling"),
            "{error}"
        );
        assert_eq!(api.group_calls.load(Ordering::SeqCst), 2);
        assert_eq!(api.kill_calls.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[test]
    fn terminal_kill_eperm_consumes_target_and_is_surfaced_after_survivor_check() {
        let _timing = serialize_probe_timing_test();
        let api = Arc::new(FaultUnixGroupApi::new(
            [InjectedGroupResult::Exists, InjectedGroupResult::Exists],
            true,
        ));
        let (_dir, child) = faulted_probe_child(
            "kill-eperm",
            "#!/bin/sh\nsleep 0.05\n",
            Arc::clone(&api) as Arc<dyn UnixGroupApi>,
        );

        let error = match finish_cleanup_work(cleanup_work_without_readers(child)) {
            Err(error) => error,
            Ok(_) => panic!("post-reap survivor suppressed deferred terminal-kill EPERM"),
        };

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert!(
            error
                .to_string()
                .contains("permission denied while terminating"),
            "{error}"
        );
        assert_eq!(api.group_calls.load(Ordering::SeqCst), 2);
        assert_eq!(api.kill_calls.load(Ordering::SeqCst), 1);
    }

    #[cfg(unix)]
    #[test]
    fn anchored_zombie_eperm_clears_only_after_post_reap_esrch() {
        let _timing = serialize_probe_timing_test();
        let api = Arc::new(FaultUnixGroupApi::new(
            [
                InjectedGroupResult::PermissionDenied,
                InjectedGroupResult::Gone,
            ],
            false,
        ));
        let (_dir, child) = faulted_probe_child(
            "zombie-only-eperm",
            "#!/bin/sh\nexit 0\n",
            Arc::clone(&api) as Arc<dyn UnixGroupApi>,
        );

        let outcome = finish_cleanup_work(cleanup_work_without_readers(child))
            .expect("post-reap ESRCH must clear zombie-only EPERM");

        assert!(outcome.status.success(), "{}", outcome.status);
        assert_eq!(api.group_calls.load(Ordering::SeqCst), 2);
        assert_eq!(api.kill_calls.load(Ordering::SeqCst), 0);
    }

    #[cfg(unix)]
    #[test]
    fn unix_leader_is_observed_without_reaping_until_group_target_is_consumed() {
        let _timing = serialize_probe_timing_test();
        let (dir, executable) = unix_script(
            "nonreaping-leader",
            "#!/bin/sh\n: > \"${0%/*}/leader-started\"\nexit 0\n",
        );
        let mut command = Command::new(executable);
        command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::null());
        let mut child = match ProbeChild::spawn(&mut command) {
            Ok(child) => child,
            Err(SpawnProbeError::BeforeSpawn(error)) => panic!("spawn grouped fixture: {error}"),
            #[cfg(windows)]
            Err(SpawnProbeError::PostSpawn { error, .. }) => {
                panic!("spawn grouped fixture after child creation: {error}")
            }
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        while !dir.path().join("leader-started").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(dir.path().join("leader-started").exists());

        let observed_deadline = Instant::now() + Duration::from_secs(2);
        while !child.try_wait_leader().expect("observe leader")
            && Instant::now() < observed_deadline
        {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            child.try_wait_leader().expect("observe exited leader"),
            "leader did not exit"
        );
        assert!(
            child
                .leader_is_waitable_without_reaping()
                .expect("query waitable leader"),
            "leader identity was reaped before process-group targeting completed"
        );

        if let Err(error) = child.kill() {
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        }
        assert!(
            child.try_wait_group().expect("consumed group state"),
            "a permission-denied terminal decision left the PGID targetable"
        );
        let status = child
            .reap_leader()
            .expect("reap leader after consuming group")
            .expect("leader exit status");
        assert!(status.success(), "{status}");
        assert!(
            !child
                .post_reap_group_exists()
                .expect("classify real zombie-only group after reap"),
            "zombie-only group still existed after reaping its identity anchor"
        );
    }

    #[cfg(unix)]
    #[test]
    fn short_lived_probe_completes_through_nonreaping_leader_path() {
        let _timing = serialize_probe_timing_test();
        let (_dir, executable) =
            unix_script("short-lived-nonreaping", "#!/bin/sh\necho 'Usage: probe'\n");
        let output = run_probe_command(
            executable.to_str().unwrap(),
            &["--help".into()],
            Duration::from_secs(2),
        )
        .expect("short-lived leader must not wedge group cleanup");
        assert!(output.status.success(), "{}", output.status);
    }

    #[test]
    fn probe_runs_current_test_binary() {
        // The test harness is a platform-native executable that supports
        // `--help`, so this exercises a real probe without relying on a
        // Unix-only command such as `true`.
        let executable = std::env::current_exe().unwrap();
        let executable = executable.to_string_lossy();
        let mut a = adapter(&executable);
        a.non_interactive_prompt_prefix_args.clear();
        a.model_flag = None;
        a.sandbox = None;
        match probe_adapter(&a, Some("--help")) {
            ProbeResult::Ok { .. } => {}
            other => panic!("expected Ok, got {:?}", DebugProbe(&other)),
        }
    }

    #[cfg(unix)]
    #[test]
    fn probe_rejects_nonzero_exit_status() {
        let (_dir, script) = unix_script("nonzero", "#!/bin/sh\nexit 23\n");
        let mut a = adapter(script.to_str().unwrap());
        a.non_interactive_prompt_prefix_args.clear();
        a.model_flag = None;
        a.sandbox = None;

        match probe_adapter(&a, Some("--help")) {
            ProbeResult::NotRunnable(msg) => {
                assert!(msg.contains("status"), "{msg}");
                assert!(msg.contains("23"), "{msg}");
            }
            other => panic!("expected NotRunnable, got {:?}", DebugProbe(&other)),
        }
    }

    #[test]
    fn probe_invocation_keeps_only_safe_leading_subcommand() {
        let mut a = adapter("runtime");
        a.non_interactive_prompt_prefix_args =
            ["run", "--", "prompt-default"].map(str::to_string).to_vec();

        let invocations = probe_invocations(&a, None);

        assert_eq!(invocations.len(), 1);
        assert_eq!(
            invocations[0].args,
            ["--model", "conjure/probe-model", "run", "--help"]
        );
        assert_eq!(
            invocations[0].required_usage.as_deref(),
            Some("runtime run")
        );
    }

    #[test]
    fn probe_invocations_use_root_help_then_version_without_recipe_tokens() {
        let mut a = adapter("copilot");
        a.non_interactive_prompt_prefix_args = ["-s", "-p"].map(str::to_string).to_vec();

        let invocations = probe_invocations(&a, None);
        let args = invocations
            .iter()
            .map(|invocation| invocation.args.as_slice())
            .collect::<Vec<_>>();

        assert_eq!(args, [vec!["--help"], vec!["--version"]]);
        assert!(invocations
            .iter()
            .all(|invocation| invocation.required_usage.is_none()));
    }

    #[test]
    fn probe_flag_override_must_be_an_option_token() {
        let mut a = adapter("definitely-not-a-real-binary-xyzzy-12345");
        a.non_interactive_prompt_prefix_args.clear();

        match probe_adapter(&a, Some("help")) {
            ProbeResult::NotRunnable(msg) => {
                assert!(msg.contains("read-only"), "{msg}");
            }
            other => panic!("expected NotRunnable, got {:?}", DebugProbe(&other)),
        }
    }

    #[test]
    fn probe_flag_override_allows_only_explicit_read_only_flags() {
        let mut a = adapter("definitely-not-a-real-binary-xyzzy-12345");
        a.non_interactive_prompt_prefix_args.clear();

        for flag in ["--help", "-h", "--version", "-V"] {
            assert!(
                matches!(probe_adapter(&a, Some(flag)), ProbeResult::NotFound),
                "approved flag `{flag}` was rejected before PATH lookup"
            );
        }
        for flag in ["-v", "--query=DO_NOT_RUN", "--delete-all"] {
            match probe_adapter(&a, Some(flag)) {
                ProbeResult::NotRunnable(msg) => {
                    assert!(msg.contains("read-only"), "{flag}: {msg}");
                }
                other => panic!(
                    "expected `{flag}` to be rejected, got {:?}",
                    DebugProbe(&other)
                ),
            }
        }
    }

    #[cfg(unix)]
    fn write_unix_script(path: &Path, body: &str) {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        fs::write(path, body).unwrap();
        let mut perms = fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms).unwrap();
    }

    #[cfg(unix)]
    fn unix_script(name: &str, body: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join(name);
        write_unix_script(&script, body);
        (dir, script)
    }

    #[cfg(unix)]
    #[test]
    fn probe_rejects_action_override_before_fixture_starts() {
        let (dir, script) = unix_script(
            "must-not-start",
            "#!/bin/sh\n: > \"$(dirname \"$0\")/started\"\nexit 0\n",
        );
        let marker = dir.path().join("started");
        let mut a = adapter(script.to_str().unwrap());
        a.non_interactive_prompt_prefix_args.clear();
        a.model_flag = None;
        a.sandbox = None;

        let result = probe_adapter(&a, Some("--query=DO_NOT_RUN"));

        assert!(
            !marker.exists(),
            "unsafe probe override spawned the fixture process"
        );
        match result {
            ProbeResult::NotRunnable(msg) => assert!(msg.contains("read-only"), "{msg}"),
            other => panic!(
                "expected unsafe override rejection, got {:?}",
                DebugProbe(&other)
            ),
        }
    }

    #[cfg(unix)]
    #[test]
    fn probe_copilot_shaped_prefix_never_enters_prompt_path() {
        let (dir, script) = unix_script(
            "copilot",
            r#"#!/bin/sh
marker_dir="${0%/*}"
if [ "$#" -eq 1 ] && [ "$1" = "--help" ]; then
  echo "Usage: copilot [OPTIONS]"
  echo "  --model <MODEL>"
  exit 0
fi
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-p" ]; then
    printf entered > "$marker_dir/prompt-entered"
    exit 0
  fi
  shift
done
echo "unexpected argv" >&2
exit 64
"#,
        );
        let prompt_marker = dir.path().join("prompt-entered");
        let mut a = adapter(script.to_str().unwrap());
        a.non_interactive_prompt_prefix_args = ["-s", "-p"].map(str::to_string).to_vec();
        a.sandbox = None;

        match probe_adapter_with_timeout(&a, None, Duration::from_secs(5)) {
            ProbeResult::Ok { probe, .. } => {
                assert!(probe.ends_with("copilot --help"), "{probe}");
            }
            other => panic!("expected Ok, got {:?}", DebugProbe(&other)),
        }
        assert!(
            !prompt_marker.exists(),
            "Copilot-shaped probe entered the prompt path"
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_rejects_same_name_binary_when_declared_subcommand_exits_nonzero() {
        let (_dir, script) = unix_script(
            "opencode",
            r#"#!/bin/sh
case "$1" in
  --version|--help)
    echo "Usage: opencode [flags]"
    exit 0
    ;;
esac
echo "unknown command: run" >&2
exit 64
"#,
        );
        let mut a = adapter(script.to_str().unwrap());
        a.non_interactive_prompt_prefix_args = vec!["run".into()];
        a.sandbox = None;

        match probe_adapter_with_timeout(&a, None, Duration::from_secs(5)) {
            ProbeResult::NotRunnable(msg) => {
                assert!(msg.contains("status"), "{msg}");
                assert!(msg.contains("64"), "{msg}");
                assert!(msg.contains("unknown command: run"), "{msg}");
            }
            other => panic!("expected NotRunnable, got {:?}", DebugProbe(&other)),
        }
    }

    #[cfg(unix)]
    #[test]
    fn probe_accepts_recipe_shaped_subcommand_help() {
        let (_dir, script) = unix_script(
            "valid-runtime",
            r#"#!/bin/sh
if [ "$#" -eq 4 ] &&
   [ "$1" = "--model" ] &&
   [ "$2" = "conjure/probe-model" ] &&
   [ "$3" = "run" ] &&
   [ "$4" = "--help" ]; then
  echo "Usage: valid-runtime run [OPTIONS]"
  echo "  --model <MODEL>"
  exit 0
fi
echo "unexpected argv: $*" >&2
exit 64
"#,
        );
        let mut a = adapter(script.to_str().unwrap());
        a.non_interactive_prompt_prefix_args = vec!["run".into()];
        a.sandbox = None;

        match probe_adapter_with_timeout(&a, None, Duration::from_secs(5)) {
            ProbeResult::Ok { probe, warnings } => {
                assert!(
                    probe.ends_with("--model conjure/probe-model run --help"),
                    "{probe}"
                );
                assert!(warnings.is_empty(), "{warnings:?}");
            }
            other => panic!("expected Ok, got {:?}", DebugProbe(&other)),
        }
    }

    #[cfg(unix)]
    #[test]
    fn probe_recipe_shape_supports_model_arg_template() {
        let (_dir, script) = unix_script(
            "template-runtime",
            r#"#!/bin/sh
if [ "$#" -eq 4 ] &&
   [ "$1" = "-c" ] &&
   [ "$2" = "model=conjure/probe-model" ] &&
   [ "$3" = "run" ] &&
   [ "$4" = "--help" ]; then
  echo "Usage: template-runtime run [OPTIONS]"
  echo "  -c model=<MODEL>"
  exit 0
fi
echo "unexpected argv: $*" >&2
exit 64
"#,
        );
        let mut a = adapter(script.to_str().unwrap());
        a.non_interactive_prompt_prefix_args = vec!["run".into()];
        a.model_flag = None;
        a.model_arg_template = Some("-c model={model}".into());
        a.sandbox = None;

        match probe_adapter_with_timeout(&a, None, Duration::from_secs(5)) {
            ProbeResult::Ok { probe, .. } => {
                assert!(
                    probe.ends_with("-c model=conjure/probe-model run --help"),
                    "{probe}"
                );
            }
            other => panic!("expected Ok, got {:?}", DebugProbe(&other)),
        }
    }

    #[cfg(unix)]
    #[test]
    fn probe_rejects_subcommand_help_without_distinguishing_usage() {
        let (_dir, script) = unix_script(
            "opencode",
            "#!/bin/sh\necho 'Usage: opencode [flags]'\nexit 0\n",
        );
        let mut a = adapter(script.to_str().unwrap());
        a.non_interactive_prompt_prefix_args = vec!["run".into()];
        a.sandbox = None;

        match probe_adapter_with_timeout(&a, None, Duration::from_secs(5)) {
            ProbeResult::NotRunnable(msg) => {
                assert!(msg.contains("opencode run"), "{msg}");
            }
            other => panic!("expected NotRunnable, got {:?}", DebugProbe(&other)),
        }
    }

    #[test]
    fn usage_signature_accepts_opencode_1_17_16_command_header() {
        assert!(has_usage_signature(
            "opencode run [message..]\n\nrun opencode with a message",
            "opencode run"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn probe_accepts_hermes_leading_subcommand_without_prompt_flags() {
        let (_dir, script) = unix_script(
            "hermes",
            r#"#!/bin/sh
if [ "$#" -eq 4 ] &&
   [ "$1" = "--model" ] &&
   [ "$2" = "conjure/probe-model" ] &&
   [ "$3" = "chat" ] &&
   [ "$4" = "--help" ]; then
  echo "Usage: hermes chat [OPTIONS]"
  echo "  --model <MODEL>"
  exit 0
fi
sleep 1
"#,
        );
        let mut a = adapter(script.to_str().unwrap());
        a.non_interactive_prompt_prefix_args = ["chat", "--source", "coven", "-Q"]
            .map(str::to_string)
            .to_vec();
        a.sandbox = None;

        match probe_adapter_with_timeout(&a, None, Duration::from_secs(5)) {
            ProbeResult::Ok { probe, .. } => {
                assert!(probe.contains("hermes"), "{probe}");
                assert!(probe.ends_with("chat --help"), "{probe}");
                assert!(!probe.contains("--source"), "{probe}");
            }
            other => panic!("expected Ok, got {:?}", DebugProbe(&other)),
        }
    }

    #[cfg(unix)]
    #[test]
    fn probe_does_not_treat_later_bare_option_value_as_subcommand() {
        let (_dir, script) = unix_script(
            "option-runtime",
            r#"#!/bin/sh
if [ "$#" -eq 1 ] && [ "$1" = "--help" ]; then
  echo "Usage: option-runtime [OPTIONS]"
  echo "  --output-format <FORMAT>"
  exit 0
fi
echo "unexpected argv: $*" >&2
exit 64
"#,
        );
        let mut a = adapter(script.to_str().unwrap());
        a.model_flag = None;
        a.non_interactive_prompt_prefix_args =
            ["--output-format", "plain"].map(str::to_string).to_vec();
        a.sandbox = None;

        match probe_adapter_with_timeout(&a, None, Duration::from_secs(5)) {
            ProbeResult::Ok { probe, warnings } => {
                assert!(probe.ends_with("option-runtime --help"), "{probe}");
                assert!(!probe.contains("--output-format"), "{probe}");
                assert!(warnings.is_empty(), "{warnings:?}");
            }
            other => panic!("expected Ok, got {:?}", DebugProbe(&other)),
        }
    }

    #[cfg(unix)]
    #[test]
    fn probe_flag_override_keeps_safe_subcommand_shape() {
        let (_dir, script) = unix_script(
            "valid-runtime",
            r#"#!/bin/sh
if [ "$#" -eq 4 ] &&
   [ "$1" = "--model" ] &&
   [ "$2" = "conjure/probe-model" ] &&
   [ "$3" = "run" ] &&
   [ "$4" = "-h" ]; then
  echo "Usage: valid-runtime run [OPTIONS]"
  echo "  --model <MODEL>"
  exit 0
fi
echo "unexpected argv: $*" >&2
exit 64
"#,
        );
        let mut a = adapter(script.to_str().unwrap());
        a.non_interactive_prompt_prefix_args = vec!["run".into()];
        a.sandbox = None;

        match probe_adapter_with_timeout(&a, Some("-h"), Duration::from_secs(5)) {
            ProbeResult::Ok { probe, .. } => {
                assert!(
                    probe.ends_with("--model conjure/probe-model run -h"),
                    "{probe}"
                );
            }
            other => panic!("expected Ok, got {:?}", DebugProbe(&other)),
        }
    }

    #[cfg(unix)]
    #[test]
    fn nonzero_probe_diagnostics_include_bounded_output_context() {
        let (_dir, script) = unix_script(
            "fails",
            r#"#!/bin/sh
echo "stdout detail"
echo "stderr detail" >&2
i=0
while [ "$i" -lt 5000 ]; do
  printf x >&2
  i=$((i + 1))
done
exit 23
"#,
        );
        let mut a = adapter(script.to_str().unwrap());
        a.non_interactive_prompt_prefix_args.clear();
        a.model_flag = None;
        a.sandbox = None;

        match probe_adapter_with_timeout(&a, Some("--help"), Duration::from_secs(5)) {
            ProbeResult::NotRunnable(msg) => {
                assert!(msg.contains("status"), "{msg}");
                assert!(msg.contains("23"), "{msg}");
                assert!(msg.contains("stdout detail"), "{msg}");
                assert!(msg.contains("stderr detail"), "{msg}");
                assert!(msg.contains("truncated"), "{msg}");
                assert!(
                    msg.len() < 3_000,
                    "diagnostic was not bounded: {}",
                    msg.len()
                );
            }
            other => panic!("expected NotRunnable, got {:?}", DebugProbe(&other)),
        }
    }

    /// Unix-only: builds a small shell script that blocks longer than the
    /// probe timeout. Windows has no equivalent one-liner executable.
    #[cfg(unix)]
    #[test]
    fn probe_times_out_blocking_binary() {
        let _timing = serialize_probe_timing_test();
        let (_dir, script) = unix_script("blocks", "#!/bin/sh\nsleep 1\n");

        let a = adapter(script.to_str().unwrap());
        let started = Instant::now();
        let result = probe_adapter_with_timeout(&a, Some("--version"), Duration::from_millis(50));
        let elapsed = started.elapsed();
        match result {
            ProbeResult::NotRunnable(msg) => assert!(msg.contains("timed out"), "{msg}"),
            other => panic!("expected NotRunnable timeout, got {:?}", DebugProbe(&other)),
        }
        assert!(
            elapsed < Duration::from_millis(500),
            "probe exceeded its timeout bound: {elapsed:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn successful_probe_cleans_up_descendant_with_closed_output_pipes() {
        let _timing = serialize_probe_timing_test();
        let (dir, executable) = unix_script(
            "descendant-runtime",
            r#"#!/bin/bash
marker_dir="${0%/*}"
(
  trap '' HUP TERM
  exec </dev/null >/dev/null 2>&1
  printf started > "$marker_dir/descendant-started"
  sleep 1
  printf fired > "$marker_dir/descendant-fired"
) &
descendant_pid=$!
disown "$descendant_pid"
while [ ! -f "$marker_dir/descendant-started" ]; do
  sleep 0.01
done
echo "Usage: descendant-runtime [OPTIONS]"
exit 0
"#,
        );
        let marker = dir.path().join("descendant-fired");
        let mut a = adapter(executable.to_str().unwrap());
        a.non_interactive_prompt_prefix_args.clear();
        a.model_flag = None;
        a.sandbox = None;

        match probe_adapter_with_timeout(&a, Some("--help"), Duration::from_secs(5)) {
            ProbeResult::Ok { probe, .. } => {
                assert!(probe.ends_with("descendant-runtime --help"), "{probe}");
            }
            other => panic!("expected Ok, got {:?}", DebugProbe(&other)),
        }
        assert!(
            dir.path().join("descendant-started").exists(),
            "fixture descendant did not start"
        );
        thread::sleep(Duration::from_millis(1_200));
        assert!(
            !marker.exists(),
            "probe returned while a process-group descendant survived"
        );
    }

    #[cfg(unix)]
    #[test]
    fn probe_timeout_kills_descendant_holding_output_pipes() {
        let _timing = serialize_probe_timing_test();
        let (dir, executable) = unix_script(
            "leaky-runtime",
            r#"#!/bin/bash
marker_dir="${0%/*}"
printf started > "$marker_dir/parent-started"
(
  trap '' HUP TERM
  printf started > "$marker_dir/descendant-started"
  sleep 4
  printf fired > "$marker_dir/descendant-fired"
) >&1 2>&2 &
descendant_pid=$!
disown "$descendant_pid"
exit 0
"#,
        );
        let marker = dir.path().join("descendant-fired");
        let args = ["--help".into()];

        let started = Instant::now();
        let result = run_probe_command(executable.to_str().unwrap(), &args, Duration::from_secs(2));
        let elapsed = started.elapsed();
        let result_label = match &result {
            Ok(output) => format!("Ok({})", output.status),
            Err(error) => format!("Err({:?}: {error})", error.kind()),
        };
        assert!(
            dir.path().join("parent-started").exists(),
            "fixture parent did not start: {result_label}"
        );
        assert!(
            dir.path().join("descendant-started").exists(),
            "fixture descendant did not start: {result_label}"
        );
        thread::sleep(Duration::from_millis(2_200));

        assert!(
            elapsed < Duration::from_secs(3),
            "probe exceeded its timeout bound while descendant retained pipes: {elapsed:?}"
        );
        assert!(
            !marker.exists(),
            "probe descendant survived the timeout and fired its marker"
        );
        match result {
            Err(error) => {
                assert_eq!(error.kind(), io::ErrorKind::TimedOut, "{error}");
                assert!(error.to_string().contains("timed out"), "{error}");
            }
            Ok(output) => panic!(
                "expected timeout while descendant retained pipes, got {}",
                output.status
            ),
        }
    }

    #[cfg(unix)]
    #[test]
    #[ignore = "helper process for cleanup_expiry_retains_ownership"]
    fn escaped_pipe_holder_helper() {
        use std::os::unix::process::CommandExt;

        let child = Command::new("sh")
            .arg("-c")
            .arg("sleep 3")
            .process_group(0)
            .spawn()
            .expect("spawn escaped pipe holder");
        drop(child);
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_expiry_retains_ownership_and_blocks_retry() {
        let _timing = serialize_probe_timing_test();
        let registry = CleanupRegistry::new();
        let executable = std::env::current_exe().unwrap();
        let module = module_path!()
            .strip_prefix("conjure::")
            .unwrap_or(module_path!());
        let helper = format!("{module}::escaped_pipe_holder_helper");
        let first_args = [
            "--exact".into(),
            helper,
            "--ignored".into(),
            "--nocapture".into(),
        ];

        let first_started = Instant::now();
        let first = run_probe_command_with_registry(
            executable.to_str().unwrap(),
            &first_args,
            Duration::from_millis(500),
            &SystemThreadSpawner,
            &registry,
            ProbeChild::kill,
        );
        let first_elapsed = first_started.elapsed();
        let first_error = match first {
            Err(error) => error,
            Ok(output) => panic!(
                "escaped pipe holder unexpectedly completed with {}",
                output.status
            ),
        };
        assert!(
            first_error.to_string().contains("cleanup exceeded"),
            "{first_error}"
        );
        assert!(
            first_elapsed < Duration::from_millis(1_500),
            "cleanup expiry exceeded its caller bound: {first_elapsed:?}"
        );

        let (dir, retry) = unix_script(
            "retry-must-not-start",
            "#!/bin/sh\n: > \"${0%/*}/retry-started\"\necho 'Usage: retry [OPTIONS]'\n",
        );
        let retry_args = ["--help".into()];
        let second_started = Instant::now();
        let second = run_probe_command_with_registry(
            retry.to_str().unwrap(),
            &retry_args,
            Duration::from_secs(1),
            &SystemThreadSpawner,
            &registry,
            ProbeChild::kill,
        );
        let second_elapsed = second_started.elapsed();

        let second_error = match second {
            Err(error) => error,
            Ok(output) => panic!(
                "retry unexpectedly started during retained cleanup and exited with {}",
                output.status
            ),
        };
        assert_eq!(
            second_error.kind(),
            io::ErrorKind::WouldBlock,
            "{second_error}"
        );
        assert!(
            second_error
                .to_string()
                .contains("previous probe cleanup is still running"),
            "{second_error}"
        );
        assert!(
            second_elapsed < Duration::from_millis(250),
            "blocked retry exceeded its caller bound: {second_elapsed:?}"
        );
        assert!(
            !dir.path().join("retry-started").exists(),
            "retry spawned another probe while cleanup workers were still owned"
        );

        let completion_deadline = Instant::now() + Duration::from_secs(5);
        let third = loop {
            let result = run_probe_command_with_registry(
                retry.to_str().unwrap(),
                &retry_args,
                Duration::from_secs(5),
                &SystemThreadSpawner,
                &registry,
                ProbeChild::kill,
            );
            if result
                .as_ref()
                .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock)
                && Instant::now() < completion_deadline
            {
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            break result.expect("completed durable cleanup must allow a later probe");
        };
        assert!(third.status.success(), "{}", third.status);
        assert!(
            dir.path().join("retry-started").exists(),
            "later probe did not start after cleanup ownership completed"
        );
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

    #[test]
    fn soft_warnings_trim_and_dedupe_whitespace_flags() {
        let mut a = adapter("probe");
        a.prompt_flag = Some("  --single  ".into());
        a.interactive_prompt_flag = Some("--single".into());
        a.system_prompt_flag = Some("   ".into());

        let warnings = soft_flag_warnings(&a, "");
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(
            warnings.iter().filter(|w| w.contains("--single")).count(),
            1,
            "trimmed duplicates must be checked once: {warnings:?}"
        );
        assert!(
            !warnings.iter().any(|w| w.contains("system-prompt")),
            "{warnings:?}"
        );
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
