//! Manifest validation.
//!
//! Enforces every rule `coven`'s `ExternalHarnessAdapterSpec::into_spec`
//! enforces today (id charset, executable shape, required label / install_hint,
//! duplicate ids, built-in collisions) **plus** the cross-field invariants the
//! new capability model introduces:
//!
//! - `capabilities.stream` requires `stream_args`.
//! - `capabilities.preassigned_session_id` requires `stream_args.session_id_flag`.
//! - a `sandbox` mapping must have a non-empty flag and both values.
//! - `model_arg_template` must contain the `{model}` placeholder.
//!
//! Validation is pure (no filesystem, no process spawning) so it runs anywhere:
//! `conjure validate`, coven's loader, and CI all share the same rules.

use crate::capabilities::Capabilities;
use crate::manifest::{AdapterManifest, RuntimeAdapter};

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
    if let Some(sandbox) = &adapter.sandbox {
        if sandbox.flag.trim().is_empty() {
            errors.push(err(tag(), "sandbox.flag", "sandbox flag must not be empty"));
        }
        if sandbox.full.trim().is_empty() {
            errors.push(err(
                tag(),
                "sandbox.full",
                "sandbox `full` value must not be empty",
            ));
        }
        if sandbox.read_only.trim().is_empty() {
            errors.push(err(
                tag(),
                "sandbox.read_only",
                "sandbox `read_only` value must not be empty",
            ));
        }
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

    if preassigned_session_id {
        let has_flag = adapter
            .stream_args
            .as_ref()
            .and_then(|a| a.session_id_flag.as_deref())
            .is_some_and(|f| !f.trim().is_empty());
        if !has_flag {
            errors.push(err(
                tag(),
                "capabilities.preassignedSessionId",
                "declares preassigned session id but no `stream_args.session_id_flag`",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::StreamArgs;
    use crate::sandbox::SandboxMapping;

    fn base_adapter(id: &str) -> RuntimeAdapter {
        RuntimeAdapter {
            id: id.into(),
            label: "Test".into(),
            executable: "test".into(),
            interactive_prompt_prefix_args: vec![],
            non_interactive_prompt_prefix_args: vec!["exec".into()],
            install_hint: "install it".into(),
            system_prompt_flag: None,
            model_flag: None,
            model_arg_template: None,
            capabilities: Capabilities::BASELINE,
            sandbox: None,
            stream_args: None,
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
            .any(|e| e.field == "capabilities.preassignedSessionId"));
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
    fn sandbox_requires_non_empty_values() {
        let mut a = base_adapter("x");
        a.sandbox = Some(SandboxMapping {
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
