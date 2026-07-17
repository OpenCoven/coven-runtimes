//! Manifest validation.
//!
//! Enforces every rule `coven`'s `ExternalHarnessAdapterSpec::into_spec`
//! enforces today (id charset, executable shape, required label / install_hint,
//! duplicate ids, built-in collisions) **plus** the cross-field invariants the
//! new capability model introduces:
//!
//! - `capabilities.stream` requires `stream_args`.
//! - `capabilities.preassigned_session_id` requires the session id flag on the
//!   active launch path: `stream_args.session_id_flag` for streaming adapters,
//!   `continuity_args.session_id_flag` otherwise.
//! - `continuity_args` must declare a usable init or resume launch, and its
//!   `session_id_flag` is dead config without `preassigned_session_id`.
//! - `event_protocol` (finite one-shot stdout) and `capabilities.stream`
//!   (long-lived stdin/stdout) are mutually exclusive.
//! - a `sandbox` mapping must have a non-empty flag and both values (flag
//!   form), or a non-empty argv list per policy (args form).
//! - `model_arg_template` must contain the `{model}` placeholder.
//!
//! Validation is pure (no filesystem, no process spawning) so it runs anywhere:
//! `conjure validate`, coven's loader, and CI all share the same rules.

use crate::capabilities::Capabilities;
use crate::manifest::{AdapterManifest, RuntimeAdapter};
use crate::sandbox::SandboxMapping;

/// Ids reserved by coven's built-in harnesses. A manifest adapter may not
/// reuse these (mirrors coven's built-in-collision check).
pub const BUILT_IN_IDS: &[&str] = &["codex", "claude"];

/// A single validation problem, tagged with the adapter id it belongs to
/// (or `None` for manifest-level issues like duplicate ids).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub adapter_id: Option<String>,
    pub field: &'static str,
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.adapter_id {
            Some(id) => write!(f, "adapter `{id}` [{}]: {}", self.field, self.message),
            None => write!(f, "manifest [{}]: {}", self.field, self.message),
        }
    }
}

/// Validate an entire manifest. Returns all problems found (not just the first)
/// so `conjure validate` can report everything in one pass.
pub fn validate_manifest(manifest: &AdapterManifest) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    if manifest.adapters.is_empty() {
        errors.push(ValidationError {
            adapter_id: None,
            field: "adapters",
            message: "manifest declares no adapters".into(),
        });
    }

    // Duplicate-id detection across the manifest.
    let mut seen: Vec<String> = Vec::new();
    for adapter in &manifest.adapters {
        let id = adapter.id.trim().to_lowercase();
        if seen.contains(&id) {
            errors.push(ValidationError {
                adapter_id: Some(id.clone()),
                field: "id",
                message: "duplicate adapter id within manifest".into(),
            });
        } else {
            seen.push(id);
        }
    }

    for adapter in &manifest.adapters {
        validate_adapter_into(adapter, &mut errors);
    }
    errors
}

/// Validate one adapter in isolation. Convenience wrapper over the internal
/// accumulator form.
pub fn validate_adapter(adapter: &RuntimeAdapter) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    validate_adapter_into(adapter, &mut errors);
    errors
}

fn validate_adapter_into(adapter: &RuntimeAdapter, errors: &mut Vec<ValidationError>) {
    let id = adapter.id.trim().to_lowercase();
    let tag = || Some(id.clone());

    // ── id ────────────────────────────────────────────────────────────────
    if id.is_empty() {
        errors.push(err(tag(), "id", "adapter id must not be empty"));
    } else if !valid_adapter_id(&id) {
        errors.push(err(
            tag(),
            "id",
            "invalid id; use lowercase letters, digits, '.', '_' or '-'",
        ));
    }
    if BUILT_IN_IDS.contains(&id.as_str()) {
        errors.push(err(
            tag(),
            "id",
            "id collides with a built-in harness (codex, claude)",
        ));
    }

    // ── executable ──────────────────────────────────────────────────────────
    let exe = adapter.executable.trim();
    if exe.is_empty() {
        errors.push(err(tag(), "executable", "executable must not be empty"));
    } else if exe.contains('/') || exe.contains('\\') || exe.chars().any(char::is_whitespace) {
        errors.push(err(
            tag(),
            "executable",
            "executable must be a bare command name (no path separators or whitespace)",
        ));
    }

    // ── required text ─────────────────────────────────────────────────────
    if adapter.label.trim().is_empty() {
        errors.push(err(tag(), "label", "label must not be empty"));
    }
    if adapter.install_hint.trim().is_empty() {
        errors.push(err(tag(), "install_hint", "install_hint must not be empty"));
    }

    if let Some(version) = adapter.version.as_deref() {
        if !valid_registry_version(version) {
            errors.push(err(
                tag(),
                "version",
                &format!("version `{version}` is not valid semver"),
            ));
        }
    }

    // ── model selection ─────────────────────────────────────────────────────
    if let Some(template) = adapter.model_arg_template.as_deref() {
        if !template.contains("{model}") {
            errors.push(err(
                tag(),
                "model_arg_template",
                "template must contain the `{model}` placeholder",
            ));
        }
    }

    // ── sandbox mapping ─────────────────────────────────────────────────────
    match &adapter.sandbox {
        Some(SandboxMapping::Flag {
            flag,
            full,
            read_only,
        }) => {
            if flag.trim().is_empty() {
                errors.push(err(tag(), "sandbox.flag", "sandbox flag must not be empty"));
            }
            if full.trim().is_empty() {
                errors.push(err(
                    tag(),
                    "sandbox.full",
                    "sandbox `full` value must not be empty",
                ));
            }
            if read_only.trim().is_empty() {
                errors.push(err(
                    tag(),
                    "sandbox.read_only",
                    "sandbox `read_only` value must not be empty",
                ));
            }
        }
        Some(SandboxMapping::Args {
            full_args,
            read_only_args,
        }) => {
            if full_args.iter().all(|t| t.trim().is_empty()) {
                errors.push(err(
                    tag(),
                    "sandbox.full_args",
                    "sandbox `full_args` must contain at least one non-empty token",
                ));
            }
            if read_only_args.iter().all(|t| t.trim().is_empty()) {
                errors.push(err(
                    tag(),
                    "sandbox.read_only_args",
                    "sandbox `read_only_args` must contain at least one non-empty token",
                ));
            }
        }
        None => {}
    }

    // ── capability cross-checks ───────────────────────────────────────────
    validate_capabilities(adapter, &id, errors);
}

fn validate_capabilities(adapter: &RuntimeAdapter, id: &str, errors: &mut Vec<ValidationError>) {
    let Capabilities {
        stream,
        preassigned_session_id,
        think: _,
        speed: _,
    } = adapter.capabilities;
    let tag = || Some(id.to_string());

    match (&adapter.stream_args, stream) {
        (None, true) => errors.push(err(
            tag(),
            "capabilities.stream",
            "declares stream but no `stream_args` provided",
        )),
        (Some(_), false) => errors.push(err(
            tag(),
            "stream_args",
            "`stream_args` provided but `capabilities.stream` is false (dead config)",
        )),
        (Some(args), true) => {
            if args.prefix_args.is_empty() {
                errors.push(err(
                    tag(),
                    "stream_args.prefix_args",
                    "stream mode requires non-empty `prefix_args`",
                ));
            }
        }
        (None, false) => {}
    }

    // A finite event bridge and a long-lived stream are distinct process
    // contracts and cannot both own stdout (mirrors coven's loader check).
    if adapter.event_protocol.is_some() && stream {
        errors.push(err(
            tag(),
            "event_protocol",
            "cannot declare both an `event_protocol` (one-shot stdout) and \
             `capabilities.stream` (long-lived stdin/stdout)",
        ));
    }

    if let Some(continuity) = &adapter.continuity_args {
        if !continuity.has_init_launch() && !continuity.has_resume_launch() {
            errors.push(err(
                tag(),
                "continuity_args",
                "provides `continuity_args` but no usable init or resume launch args",
            ));
        }
        if continuity.session_id_flag().is_some() && !preassigned_session_id {
            errors.push(err(
                tag(),
                "continuity_args.session_id_flag",
                "provides `continuity_args.session_id_flag` but \
                 `capabilities.preassigned_session_id` is false (dead config)",
            ));
        }
    }

    if preassigned_session_id {
        let stream_flag = adapter
            .stream_args
            .as_ref()
            .and_then(|a| a.session_id_flag.as_deref())
            .is_some_and(|f| !f.trim().is_empty());
        let continuity_flag = adapter
            .continuity_args
            .as_ref()
            .and_then(|a| a.session_id_flag())
            .is_some();
        // The flag must live on the launch path that actually runs: a
        // streaming adapter receives the pre-assigned id through
        // `stream_args`, so a continuity-only flag would validate here but
        // never reach the streaming process.
        if stream && !stream_flag {
            errors.push(err(
                tag(),
                "capabilities.preassigned_session_id",
                "declares preassigned session id with stream mode but no \
                 `stream_args.session_id_flag` (a streaming launch cannot \
                 receive the id through `continuity_args`)",
            ));
        } else if !stream && !continuity_flag {
            errors.push(err(
                tag(),
                "capabilities.preassigned_session_id",
                "declares preassigned session id but no \
                 `continuity_args.session_id_flag`",
            ));
        }
    }
}

fn err(adapter_id: Option<String>, field: &'static str, message: &str) -> ValidationError {
    ValidationError {
        adapter_id,
        field,
        message: message.to_string(),
    }
}

/// Adapter id charset check, matching coven's `valid_adapter_id`:
/// non-empty, ASCII, and every char is a lowercase letter, digit, `.`, `_`, or `-`.
pub fn valid_adapter_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-'
        })
}

/// Minimal semver accepted by the registry: exactly `major.minor.patch`, with
/// numeric components. Pre-release and build metadata are not ordered yet.
pub fn valid_registry_version(value: &str) -> bool {
    let mut parts = value.trim().split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    let Some(minor) = parts.next() else {
        return false;
    };
    let Some(patch) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && [major, minor, patch]
            .iter()
            .all(|part| !part.is_empty() && part.parse::<u64>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ContinuityArgs, EventProtocol, StreamArgs};
    use crate::sandbox::SandboxMapping;

    fn base_adapter(id: &str) -> RuntimeAdapter {
        RuntimeAdapter {
            id: id.into(),
            label: "Test".into(),
            executable: "test".into(),
            interactive_prompt_prefix_args: vec![],
            non_interactive_prompt_prefix_args: vec!["exec".into()],
            prompt_flag: None,
            interactive_prompt_flag: None,
            install_hint: "install it".into(),
            system_prompt_flag: None,
            model_flag: None,
            model_arg_template: None,
            capabilities: Capabilities::BASELINE,
            sandbox: None,
            stream_args: None,
            continuity_args: None,
            event_protocol: None,
            version: None,
            homepage: None,
            description: None,
        }
    }

    #[test]
    fn baseline_adapter_is_valid() {
        assert!(validate_adapter(&base_adapter("hermes")).is_empty());
    }

    #[test]
    fn rejects_built_in_id_collision() {
        let errs = validate_adapter(&base_adapter("codex"));
        assert!(errs.iter().any(|e| e.field == "id"));
    }

    #[test]
    fn rejects_bad_id_charset() {
        let errs = validate_adapter(&base_adapter("Bad Id!"));
        assert!(errs.iter().any(|e| e.field == "id"));
    }

    #[test]
    fn rejects_executable_with_path_or_space() {
        let mut a = base_adapter("x");
        a.executable = "bin/x".into();
        assert!(validate_adapter(&a).iter().any(|e| e.field == "executable"));

        let mut b = base_adapter("y");
        b.executable = "two words".into();
        assert!(validate_adapter(&b).iter().any(|e| e.field == "executable"));
    }

    #[test]
    fn rejects_empty_label_and_hint() {
        let mut a = base_adapter("x");
        a.label = "  ".into();
        a.install_hint = "".into();
        let errs = validate_adapter(&a);
        assert!(errs.iter().any(|e| e.field == "label"));
        assert!(errs.iter().any(|e| e.field == "install_hint"));
    }

    #[test]
    fn rejects_non_semver_version() {
        let mut a = base_adapter("x");
        a.version = Some("not-semver".into());
        let errs = validate_adapter(&a);
        assert!(errs
            .iter()
            .any(|e| e.field == "version" && e.message.contains("not valid semver")));
    }

    #[test]
    fn stream_requires_stream_args() {
        let mut a = base_adapter("x");
        a.capabilities.stream = true;
        let errs = validate_adapter(&a);
        assert!(errs.iter().any(|e| e.field == "capabilities.stream"));
    }

    #[test]
    fn stream_args_without_capability_is_dead_config() {
        let mut a = base_adapter("x");
        a.stream_args = Some(StreamArgs {
            prefix_args: vec!["-p".into()],
            session_id_flag: None,
            resume_flag: None,
        });
        let errs = validate_adapter(&a);
        assert!(errs.iter().any(|e| e.field == "stream_args"));
    }

    #[test]
    fn preassigned_session_requires_flag() {
        let mut a = base_adapter("x");
        a.capabilities.stream = true;
        a.capabilities.preassigned_session_id = true;
        a.stream_args = Some(StreamArgs {
            prefix_args: vec!["-p".into()],
            session_id_flag: None,
            resume_flag: None,
        });
        let errs = validate_adapter(&a);
        assert!(errs
            .iter()
            .any(|e| e.field == "capabilities.preassigned_session_id"));
    }

    /// A streaming adapter must carry the session flag in `stream_args`; a
    /// continuity-only flag cannot reach a streaming launch (Codex review).
    #[test]
    fn preassigned_streaming_session_rejects_continuity_only_flag() {
        let mut a = base_adapter("x");
        a.capabilities.stream = true;
        a.capabilities.preassigned_session_id = true;
        a.stream_args = Some(StreamArgs {
            prefix_args: vec!["-p".into()],
            session_id_flag: None,
            resume_flag: None,
        });
        a.continuity_args = Some(ContinuityArgs {
            init_prefix_args: vec!["--print".into()],
            resume_prefix_args: vec!["--print".into()],
            session_id_flag: Some("--session-id".into()),
            resume_flag: Some("--resume".into()),
        });
        let errs = validate_adapter(&a);
        assert!(errs
            .iter()
            .any(|e| e.field == "capabilities.preassigned_session_id"
                && e.message.contains("stream_args.session_id_flag")));
    }

    /// A continuity-only session-id flag (no stream mode at all) satisfies the
    /// preassigned-session requirement — the Grok Build shape.
    #[test]
    fn preassigned_session_accepts_continuity_flag() {
        let mut a = base_adapter("grok");
        a.capabilities.preassigned_session_id = true;
        a.event_protocol = Some(EventProtocol::GrokHeadlessV1);
        a.continuity_args = Some(ContinuityArgs {
            init_prefix_args: vec!["--output-format".into(), "streaming-json".into()],
            resume_prefix_args: vec!["--output-format".into(), "streaming-json".into()],
            session_id_flag: Some("--session-id".into()),
            resume_flag: Some("--resume".into()),
        });
        assert!(
            validate_adapter(&a).is_empty(),
            "{:?}",
            validate_adapter(&a)
        );
    }

    #[test]
    fn event_protocol_conflicts_with_stream() {
        let mut a = base_adapter("x");
        a.capabilities.stream = true;
        a.stream_args = Some(StreamArgs {
            prefix_args: vec!["-p".into()],
            session_id_flag: None,
            resume_flag: None,
        });
        a.event_protocol = Some(EventProtocol::GrokHeadlessV1);
        let errs = validate_adapter(&a);
        assert!(errs
            .iter()
            .any(|e| e.field == "event_protocol" && e.message.contains("cannot declare both")));
    }

    #[test]
    fn continuity_args_require_a_usable_launch() {
        let mut a = base_adapter("x");
        a.continuity_args = Some(ContinuityArgs {
            init_prefix_args: vec!["  ".into()],
            resume_prefix_args: vec![],
            session_id_flag: None,
            resume_flag: None,
        });
        let errs = validate_adapter(&a);
        assert!(errs
            .iter()
            .any(|e| e.field == "continuity_args" && e.message.contains("no usable init")));
    }

    #[test]
    fn continuity_session_flag_without_capability_is_dead_config() {
        let mut a = base_adapter("x");
        a.continuity_args = Some(ContinuityArgs {
            init_prefix_args: vec![],
            resume_prefix_args: vec![],
            session_id_flag: Some("--session-id".into()),
            resume_flag: None,
        });
        let errs = validate_adapter(&a);
        assert!(errs
            .iter()
            .any(|e| e.field == "continuity_args.session_id_flag"
                && e.message.contains("dead config")));
    }

    #[test]
    fn valid_streaming_adapter_passes() {
        let mut a = base_adapter("aria");
        a.capabilities.stream = true;
        a.capabilities.preassigned_session_id = true;
        a.stream_args = Some(StreamArgs {
            prefix_args: vec!["-p".into(), "stream-json".into()],
            session_id_flag: Some("--session-id".into()),
            resume_flag: Some("--resume".into()),
        });
        assert!(
            validate_adapter(&a).is_empty(),
            "{:?}",
            validate_adapter(&a)
        );
    }

    #[test]
    fn model_template_requires_placeholder() {
        let mut a = base_adapter("x");
        a.model_arg_template = Some("-c model=fixed".into());
        assert!(validate_adapter(&a)
            .iter()
            .any(|e| e.field == "model_arg_template"));

        a.model_arg_template = Some("-c model={model}".into());
        assert!(!validate_adapter(&a)
            .iter()
            .any(|e| e.field == "model_arg_template"));
    }

    #[test]
    fn sandbox_flag_form_requires_non_empty_values() {
        let mut a = base_adapter("x");
        a.sandbox = Some(SandboxMapping::Flag {
            flag: "".into(),
            full: "".into(),
            read_only: "".into(),
        });
        let errs = validate_adapter(&a);
        assert!(errs.iter().any(|e| e.field == "sandbox.flag"));
        assert!(errs.iter().any(|e| e.field == "sandbox.full"));
        assert!(errs.iter().any(|e| e.field == "sandbox.read_only"));
    }

    #[test]
    fn sandbox_args_form_requires_non_empty_lists() {
        let mut a = base_adapter("x");
        a.sandbox = Some(SandboxMapping::Args {
            full_args: vec![],
            read_only_args: vec!["  ".into()],
        });
        let errs = validate_adapter(&a);
        assert!(errs.iter().any(|e| e.field == "sandbox.full_args"));
        assert!(errs.iter().any(|e| e.field == "sandbox.read_only_args"));
    }

    #[test]
    fn sandbox_args_form_valid_passes() {
        let mut a = base_adapter("copilot-test");
        a.sandbox = Some(SandboxMapping::Args {
            full_args: vec!["--allow-all".into()],
            read_only_args: vec!["--deny-tool".into(), "write".into()],
        });
        assert!(
            validate_adapter(&a).is_empty(),
            "{:?}",
            validate_adapter(&a)
        );
    }

    #[test]
    fn detects_duplicate_ids_in_manifest() {
        let manifest = AdapterManifest {
            adapters: vec![base_adapter("dup"), base_adapter("dup")],
        };
        let errs = validate_manifest(&manifest);
        assert!(errs
            .iter()
            .any(|e| e.field == "id" && e.message.contains("duplicate")));
    }

    #[test]
    fn empty_manifest_is_flagged() {
        let errs = validate_manifest(&AdapterManifest { adapters: vec![] });
        assert!(errs.iter().any(|e| e.field == "adapters"));
    }
}
