//! The studio form's field catalog — the single source of truth mapping every
//! editable [`RuntimeAdapter`] field to a label, section, input kind, help
//! line, validation tags, and get/set accessors.
//!
//! The catalog is static; which rows are *visible* depends on the adapter's
//! current sandbox form (see [`visible_fields`]). Accessors are total: setters
//! accept any string and normalize (trim, empty ⇒ `None`, whitespace-split
//! argv), so the state machine never has to reject input mid-edit.

use coven_runtime_spec::{ContinuityArgs, RuntimeAdapter, SandboxMapping, StreamArgs};

/// Form section, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    Identity,
    Launch,
    Prompt,
    Model,
    Capabilities,
    Sandbox,
    Stream,
    Continuity,
}

impl Section {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Section::Identity => "Identity",
            Section::Launch => "Launch",
            Section::Prompt => "Prompt binding",
            Section::Model => "Model",
            Section::Capabilities => "Capabilities",
            Section::Sandbox => "Sandbox",
            Section::Stream => "Stream args",
            Section::Continuity => "Continuity args",
        }
    }
}

/// How a field is edited and rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldKind {
    /// Free text, stored as-is (trimmed).
    Text,
    /// Free text where blank means "not declared" (`None`).
    OptText,
    /// Space-separated argv tokens.
    Args,
    /// Boolean capability toggle (Space / Enter flips).
    Bool,
    /// The sandbox form selector: cycles none → flag → args.
    SandboxKind,
    /// Model id forwarding: cycles strip_provider ↔ preserve.
    ModelIdTransform,
}

/// One editable field: metadata plus total get/set accessors.
pub(crate) struct FieldSpec {
    pub(crate) label: &'static str,
    pub(crate) section: Section,
    pub(crate) kind: FieldKind,
    pub(crate) help: &'static str,
    /// `ValidationError::field` tags that should highlight this row.
    pub(crate) error_keys: &'static [&'static str],
    pub(crate) get: fn(&RuntimeAdapter) -> String,
    pub(crate) set: fn(&mut RuntimeAdapter, &str),
}

// ── accessor helpers ─────────────────────────────────────────────────────────

fn opt(value: &Option<String>) -> String {
    value.clone().unwrap_or_default()
}

fn set_opt(input: &str) -> Option<String> {
    let trimmed = input.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn args_to_string(args: &[String]) -> String {
    args.join(" ")
}

fn parse_args(input: &str) -> Vec<String> {
    input.split_whitespace().map(str::to_string).collect()
}

fn bool_label(on: bool) -> String {
    if on { "true" } else { "false" }.to_string()
}

/// Drop `stream_args` when every field is empty — blank form rows mean "not
/// declared", matching the Option semantics of the manifest.
fn normalize_stream(adapter: &mut RuntimeAdapter) {
    if let Some(s) = &adapter.stream_args {
        if s.prefix_args.is_empty() && s.session_id_flag.is_none() && s.resume_flag.is_none() {
            adapter.stream_args = None;
        }
    }
}

fn stream_mut(adapter: &mut RuntimeAdapter) -> &mut StreamArgs {
    adapter.stream_args.get_or_insert_with(|| StreamArgs {
        prefix_args: Vec::new(),
        session_id_flag: None,
        resume_flag: None,
    })
}

/// Drop `continuity_args` when every field is empty (mirror of
/// [`normalize_stream`]).
fn normalize_continuity(adapter: &mut RuntimeAdapter) {
    if let Some(c) = &adapter.continuity_args {
        if c.init_prefix_args.is_empty()
            && c.resume_prefix_args.is_empty()
            && c.session_id_flag.is_none()
            && c.resume_flag.is_none()
        {
            adapter.continuity_args = None;
        }
    }
}

fn continuity_mut(adapter: &mut RuntimeAdapter) -> &mut ContinuityArgs {
    adapter
        .continuity_args
        .get_or_insert_with(|| ContinuityArgs {
            init_prefix_args: Vec::new(),
            resume_prefix_args: Vec::new(),
            session_id_flag: None,
            resume_flag: None,
        })
}

/// The sandbox form currently selected, as the selector row's display value.
pub(crate) fn sandbox_kind(adapter: &RuntimeAdapter) -> &'static str {
    match &adapter.sandbox {
        None => "none",
        Some(SandboxMapping::Flag { .. }) => "flag",
        Some(SandboxMapping::Args { .. }) => "args",
    }
}

/// Cycle the sandbox form: none → flag → args → none. Field values are reset
/// on each transition — the two forms don't share shape.
pub(crate) fn cycle_sandbox(adapter: &mut RuntimeAdapter) {
    adapter.sandbox = match adapter.sandbox.take() {
        None => Some(SandboxMapping::Flag {
            flag: String::new(),
            full: String::new(),
            read_only: String::new(),
        }),
        Some(SandboxMapping::Flag { .. }) => Some(SandboxMapping::Args {
            full_args: Vec::new(),
            read_only_args: Vec::new(),
        }),
        Some(SandboxMapping::Args { .. }) => None,
    };
}

/// Cycle how provider-qualified model ids are forwarded to the runtime.
pub(crate) fn cycle_model_id_transform(adapter: &mut RuntimeAdapter) {
    adapter.model_id_transform = adapter.model_id_transform.toggled();
}

// ── the catalog ──────────────────────────────────────────────────────────────

pub(crate) static IDENTITY_AND_CORE: &[FieldSpec] = &[
    FieldSpec {
        label: "id",
        section: Section::Identity,
        kind: FieldKind::Text,
        help: "Canonical id: lowercase letters, digits, '.', '_', '-'. Not `codex`/`claude`.",
        error_keys: &["id"],
        get: |a| a.id.clone(),
        set: |a, v| a.id = v.trim().to_string(),
    },
    FieldSpec {
        label: "label",
        section: Section::Identity,
        kind: FieldKind::Text,
        help: "Human display name shown in pickers, e.g. `Hermes Agent`.",
        error_keys: &["label"],
        get: |a| a.label.clone(),
        set: |a, v| a.label = v.trim().to_string(),
    },
    FieldSpec {
        label: "executable",
        section: Section::Identity,
        kind: FieldKind::Text,
        help: "Bare command name on PATH — no path separators, no whitespace.",
        error_keys: &["executable"],
        get: |a| a.executable.clone(),
        set: |a, v| a.executable = v.trim().to_string(),
    },
    FieldSpec {
        label: "install_hint",
        section: Section::Identity,
        kind: FieldKind::Text,
        help: "What `coven doctor` prints when the binary is missing: install URL + verify command.",
        error_keys: &["install_hint"],
        get: |a| a.install_hint.clone(),
        set: |a, v| a.install_hint = v.trim().to_string(),
    },
    FieldSpec {
        label: "version",
        section: Section::Identity,
        kind: FieldKind::OptText,
        help: "Semver of this adapter definition; required for registry acceptance.",
        error_keys: &["version"],
        get: |a| opt(&a.version),
        set: |a, v| a.version = set_opt(v),
    },
    FieldSpec {
        label: "homepage",
        section: Section::Identity,
        kind: FieldKind::OptText,
        help: "Project homepage / docs URL (registry metadata).",
        error_keys: &[],
        get: |a| opt(&a.homepage),
        set: |a, v| a.homepage = set_opt(v),
    },
    FieldSpec {
        label: "description",
        section: Section::Identity,
        kind: FieldKind::OptText,
        help: "One-line description for registry listings.",
        error_keys: &[],
        get: |a| opt(&a.description),
        set: |a, v| a.description = set_opt(v),
    },
    FieldSpec {
        label: "interactive prefix args",
        section: Section::Launch,
        kind: FieldKind::Args,
        help: "argv prefix for an interactive launch (space-separated; prompt appended last).",
        error_keys: &["interactive_prompt_prefix_args"],
        get: |a| args_to_string(&a.interactive_prompt_prefix_args),
        set: |a, v| a.interactive_prompt_prefix_args = parse_args(v),
    },
    FieldSpec {
        label: "non-interactive prefix args",
        section: Section::Launch,
        kind: FieldKind::Args,
        help: "argv prefix for a one-shot run (space-separated), e.g. `run` for `opencode run <prompt>`.",
        error_keys: &["non_interactive_prompt_prefix_args"],
        get: |a| args_to_string(&a.non_interactive_prompt_prefix_args),
        set: |a, v| a.non_interactive_prompt_prefix_args = parse_args(v),
    },
    FieldSpec {
        label: "prompt_flag",
        section: Section::Prompt,
        kind: FieldKind::OptText,
        help: "Flag binding the one-shot prompt (e.g. `--single`); blank = prompt is positional.",
        error_keys: &["prompt_flag"],
        get: |a| opt(&a.prompt_flag),
        set: |a, v| a.prompt_flag = set_opt(v),
    },
    FieldSpec {
        label: "interactive_prompt_flag",
        section: Section::Prompt,
        kind: FieldKind::OptText,
        help: "Flag binding the prompt on interactive launches; blank = falls back to prompt_flag.",
        error_keys: &["interactive_prompt_flag"],
        get: |a| opt(&a.interactive_prompt_flag),
        set: |a, v| a.interactive_prompt_flag = set_opt(v),
    },
    FieldSpec {
        label: "system_prompt_flag",
        section: Section::Prompt,
        kind: FieldKind::OptText,
        help: "Flag injecting a system prompt (e.g. `--append-system-prompt`); blank = preamble fallback.",
        error_keys: &["system_prompt_flag"],
        get: |a| opt(&a.system_prompt_flag),
        set: |a, v| a.system_prompt_flag = set_opt(v),
    },
    FieldSpec {
        label: "model_flag",
        section: Section::Model,
        kind: FieldKind::OptText,
        help: "Simple `--flag <value>` model selector (e.g. `--model`).",
        error_keys: &["model_flag"],
        get: |a| opt(&a.model_flag),
        set: |a, v| a.model_flag = set_opt(v),
    },
    FieldSpec {
        label: "model_arg_template",
        section: Section::Model,
        kind: FieldKind::OptText,
        help: "argv template for non-trivial selection; must contain `{model}`. Overrides model_flag.",
        error_keys: &["model_arg_template"],
        get: |a| opt(&a.model_arg_template),
        set: |a, v| a.model_arg_template = set_opt(v),
    },
    FieldSpec {
        label: "model_id_transform",
        section: Section::Model,
        kind: FieldKind::ModelIdTransform,
        help: "Space/Enter cycles strip_provider ↔ preserve; preserve requires model_flag or model_arg_template.",
        error_keys: &["model_id_transform"],
        get: |a| a.model_id_transform.as_str().to_string(),
        set: |_, _| {},
    },
    FieldSpec {
        label: "stream",
        section: Section::Capabilities,
        kind: FieldKind::Bool,
        help: "Persistent stream-JSON mode. Requires stream args below. Never claim untested!",
        error_keys: &["capabilities.stream", "capabilities"],
        get: |a| bool_label(a.capabilities.stream),
        set: |a, v| a.capabilities.stream = v == "true",
    },
    FieldSpec {
        label: "preassigned_session_id",
        section: Section::Capabilities,
        kind: FieldKind::Bool,
        help: "Runtime accepts a pre-assigned session id (stream or continuity session_id_flag).",
        error_keys: &["capabilities.preassigned_session_id"],
        get: |a| bool_label(a.capabilities.preassigned_session_id),
        set: |a, v| a.capabilities.preassigned_session_id = v == "true",
    },
    FieldSpec {
        label: "think",
        section: Section::Capabilities,
        kind: FieldKind::Bool,
        help: "Runtime honors a think/reasoning toggle.",
        error_keys: &[],
        get: |a| bool_label(a.capabilities.think),
        set: |a, v| a.capabilities.think = v == "true",
    },
    FieldSpec {
        label: "speed",
        section: Section::Capabilities,
        kind: FieldKind::Bool,
        help: "Runtime honors a speed/effort toggle.",
        error_keys: &[],
        get: |a| bool_label(a.capabilities.speed),
        set: |a, v| a.capabilities.speed = v == "true",
    },
];

/// The sandbox form selector row.
pub(crate) static SANDBOX_KIND_FIELD: FieldSpec = FieldSpec {
    label: "sandbox form",
    section: Section::Sandbox,
    kind: FieldKind::SandboxKind,
    help: "Space cycles: none → flag (one flag+value per policy) → args (whole argv per policy).",
    error_keys: &["sandbox"],
    get: |a| sandbox_kind(a).to_string(),
    set: |_, _| {},
};

/// Rows shown when the sandbox is in flag form.
pub(crate) static SANDBOX_FLAG_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        label: "sandbox flag",
        section: Section::Sandbox,
        kind: FieldKind::Text,
        help: "The permission flag, e.g. `--permission-mode` (Claude) or `--sandbox` (Codex).",
        error_keys: &["sandbox.flag"],
        get: |a| match &a.sandbox {
            Some(SandboxMapping::Flag { flag, .. }) => flag.clone(),
            _ => String::new(),
        },
        set: |a, v| {
            if let Some(SandboxMapping::Flag { flag, .. }) = &mut a.sandbox {
                *flag = v.trim().to_string();
            }
        },
    },
    FieldSpec {
        label: "full value",
        section: Section::Sandbox,
        kind: FieldKind::Text,
        help: "Value passed for the full (unrestricted) policy, e.g. `bypassPermissions`.",
        error_keys: &["sandbox.full"],
        get: |a| match &a.sandbox {
            Some(SandboxMapping::Flag { full, .. }) => full.clone(),
            _ => String::new(),
        },
        set: |a, v| {
            if let Some(SandboxMapping::Flag { full, .. }) = &mut a.sandbox {
                *full = v.trim().to_string();
            }
        },
    },
    FieldSpec {
        label: "read-only value",
        section: Section::Sandbox,
        kind: FieldKind::Text,
        help: "Value passed for the read-only policy, e.g. `plan` or `read-only`.",
        error_keys: &["sandbox.read_only"],
        get: |a| match &a.sandbox {
            Some(SandboxMapping::Flag { read_only, .. }) => read_only.clone(),
            _ => String::new(),
        },
        set: |a, v| {
            if let Some(SandboxMapping::Flag { read_only, .. }) = &mut a.sandbox {
                *read_only = v.trim().to_string();
            }
        },
    },
];

/// Rows shown when the sandbox is in args form.
pub(crate) static SANDBOX_ARGS_FIELDS: &[FieldSpec] = &[
    FieldSpec {
        label: "full args",
        section: Section::Sandbox,
        kind: FieldKind::Args,
        help: "argv for the full policy (space-separated), e.g. `--allow-all`.",
        error_keys: &["sandbox.full_args"],
        get: |a| match &a.sandbox {
            Some(SandboxMapping::Args { full_args, .. }) => args_to_string(full_args),
            _ => String::new(),
        },
        set: |a, v| {
            if let Some(SandboxMapping::Args { full_args, .. }) = &mut a.sandbox {
                *full_args = parse_args(v);
            }
        },
    },
    FieldSpec {
        label: "read-only args",
        section: Section::Sandbox,
        kind: FieldKind::Args,
        help: "argv for the read-only policy, e.g. `--deny-tool write --deny-tool shell`.",
        error_keys: &["sandbox.read_only_args"],
        get: |a| match &a.sandbox {
            Some(SandboxMapping::Args { read_only_args, .. }) => args_to_string(read_only_args),
            _ => String::new(),
        },
        set: |a, v| {
            if let Some(SandboxMapping::Args { read_only_args, .. }) = &mut a.sandbox {
                *read_only_args = parse_args(v);
            }
        },
    },
];

pub(crate) static STREAM_AND_CONTINUITY: &[FieldSpec] = &[
    FieldSpec {
        label: "stream prefix args",
        section: Section::Stream,
        kind: FieldKind::Args,
        help: "argv that enters persistent stream-JSON mode. Blank all three rows = not declared.",
        error_keys: &["stream_args", "stream_args.prefix_args"],
        get: |a| {
            a.stream_args
                .as_ref()
                .map(|s| args_to_string(&s.prefix_args))
                .unwrap_or_default()
        },
        set: |a, v| {
            stream_mut(a).prefix_args = parse_args(v);
            normalize_stream(a);
        },
    },
    FieldSpec {
        label: "stream session-id flag",
        section: Section::Stream,
        kind: FieldKind::OptText,
        help: "Flag pre-assigning the session id at stream launch, e.g. `--session-id`.",
        error_keys: &["stream_args.session_id_flag"],
        get: |a| {
            a.stream_args
                .as_ref()
                .map(|s| opt(&s.session_id_flag))
                .unwrap_or_default()
        },
        set: |a, v| {
            stream_mut(a).session_id_flag = set_opt(v);
            normalize_stream(a);
        },
    },
    FieldSpec {
        label: "stream resume flag",
        section: Section::Stream,
        kind: FieldKind::OptText,
        help: "Flag resuming an existing stream session, e.g. `--resume`.",
        error_keys: &["stream_args.resume_flag"],
        get: |a| {
            a.stream_args
                .as_ref()
                .map(|s| opt(&s.resume_flag))
                .unwrap_or_default()
        },
        set: |a, v| {
            stream_mut(a).resume_flag = set_opt(v);
            normalize_stream(a);
        },
    },
    FieldSpec {
        label: "continuity init args",
        section: Section::Continuity,
        kind: FieldKind::Args,
        help: "argv prefix initializing a fresh named conversation. Blank all four rows = not declared.",
        error_keys: &["continuity_args"],
        get: |a| {
            a.continuity_args
                .as_ref()
                .map(|c| args_to_string(&c.init_prefix_args))
                .unwrap_or_default()
        },
        set: |a, v| {
            continuity_mut(a).init_prefix_args = parse_args(v);
            normalize_continuity(a);
        },
    },
    FieldSpec {
        label: "continuity resume args",
        section: Section::Continuity,
        kind: FieldKind::Args,
        help: "argv prefix resuming an existing conversation.",
        error_keys: &["continuity_args"],
        get: |a| {
            a.continuity_args
                .as_ref()
                .map(|c| args_to_string(&c.resume_prefix_args))
                .unwrap_or_default()
        },
        set: |a, v| {
            continuity_mut(a).resume_prefix_args = parse_args(v);
            normalize_continuity(a);
        },
    },
    FieldSpec {
        label: "continuity session-id flag",
        section: Section::Continuity,
        kind: FieldKind::OptText,
        help: "Flag pre-assigning the session id on a cold start, e.g. `--session-id`.",
        error_keys: &["continuity_args.session_id_flag"],
        get: |a| {
            a.continuity_args
                .as_ref()
                .map(|c| opt(&c.session_id_flag))
                .unwrap_or_default()
        },
        set: |a, v| {
            continuity_mut(a).session_id_flag = set_opt(v);
            normalize_continuity(a);
        },
    },
    FieldSpec {
        label: "continuity resume flag",
        section: Section::Continuity,
        kind: FieldKind::OptText,
        help: "Flag resuming an existing session, e.g. `--resume`.",
        error_keys: &["continuity_args.resume_flag"],
        get: |a| {
            a.continuity_args
                .as_ref()
                .map(|c| opt(&c.resume_flag))
                .unwrap_or_default()
        },
        set: |a, v| {
            continuity_mut(a).resume_flag = set_opt(v);
            normalize_continuity(a);
        },
    },
];

/// The rows visible for the adapter's current shape, in form order.
pub(crate) fn visible_fields(adapter: &RuntimeAdapter) -> Vec<&'static FieldSpec> {
    let mut fields: Vec<&'static FieldSpec> = IDENTITY_AND_CORE.iter().collect();
    fields.push(&SANDBOX_KIND_FIELD);
    match &adapter.sandbox {
        None => {}
        Some(SandboxMapping::Flag { .. }) => fields.extend(SANDBOX_FLAG_FIELDS.iter()),
        Some(SandboxMapping::Args { .. }) => fields.extend(SANDBOX_ARGS_FIELDS.iter()),
    }
    fields.extend(STREAM_AND_CONTINUITY.iter());
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use coven_runtime_spec::{Capabilities, ModelIdTransform};

    fn full_adapter() -> RuntimeAdapter {
        RuntimeAdapter {
            id: "grok".into(),
            label: "Grok Build".into(),
            executable: "grok".into(),
            interactive_prompt_prefix_args: vec!["--no-alt-screen".into()],
            non_interactive_prompt_prefix_args: vec!["--output-format".into(), "plain".into()],
            prompt_flag: Some("--single".into()),
            interactive_prompt_flag: Some("--single".into()),
            install_hint: "Install Grok Build.".into(),
            system_prompt_flag: Some("--rules".into()),
            model_flag: Some("--model".into()),
            model_arg_template: None,
            model_id_transform: ModelIdTransform::StripProvider,
            capabilities: Capabilities {
                stream: false,
                preassigned_session_id: true,
                think: false,
                speed: false,
            },
            sandbox: Some(SandboxMapping::Args {
                full_args: vec!["--permission-mode".into(), "bypassPermissions".into()],
                read_only_args: vec!["--sandbox".into(), "read-only".into()],
            }),
            stream_args: None,
            continuity_args: Some(ContinuityArgs {
                init_prefix_args: vec!["run".into()],
                resume_prefix_args: vec!["run".into()],
                session_id_flag: Some("--session-id".into()),
                resume_flag: Some("--resume".into()),
            }),
            version: Some("1.0.0".into()),
            homepage: Some("https://example.com".into()),
            description: Some("desc".into()),
        }
    }

    /// Every visible field's set(get(a)) must be the identity — the form can
    /// round-trip any adapter without corrupting it.
    #[test]
    fn every_field_round_trips() {
        let original = full_adapter();
        for field in visible_fields(&original) {
            let mut copy = original.clone();
            let value = (field.get)(&original);
            (field.set)(&mut copy, &value);
            assert_eq!(copy, original, "field `{}` did not round-trip", field.label);
        }
    }

    #[test]
    fn opt_text_blank_clears_to_none() {
        let mut a = full_adapter();
        let field = IDENTITY_AND_CORE
            .iter()
            .find(|f| f.label == "version")
            .unwrap();
        (field.set)(&mut a, "   ");
        assert_eq!(a.version, None);
    }

    #[test]
    fn args_fields_split_on_whitespace() {
        let mut a = full_adapter();
        let field = IDENTITY_AND_CORE
            .iter()
            .find(|f| f.label == "non-interactive prefix args")
            .unwrap();
        (field.set)(&mut a, "  run   --fast  ");
        assert_eq!(a.non_interactive_prompt_prefix_args, vec!["run", "--fast"]);
    }

    /// Blanking every stream row must drop `stream_args` entirely, and filling
    /// any row must materialize it.
    #[test]
    fn stream_rows_normalize_option() {
        let mut a = full_adapter();
        let prefix = STREAM_AND_CONTINUITY
            .iter()
            .find(|f| f.label == "stream prefix args")
            .unwrap();
        (prefix.set)(&mut a, "-p --output-format stream-json");
        assert!(a.stream_args.is_some());
        (prefix.set)(&mut a, "");
        assert!(a.stream_args.is_none());
    }

    #[test]
    fn continuity_rows_normalize_option() {
        let mut a = full_adapter();
        for field in STREAM_AND_CONTINUITY
            .iter()
            .filter(|f| f.section == Section::Continuity)
        {
            (field.set)(&mut a, "");
        }
        assert!(a.continuity_args.is_none());
        let sid = STREAM_AND_CONTINUITY
            .iter()
            .find(|f| f.label == "continuity session-id flag")
            .unwrap();
        (sid.set)(&mut a, "--session-id");
        assert_eq!(
            a.continuity_args.as_ref().unwrap().session_id_flag(),
            Some("--session-id")
        );
    }

    /// The sandbox selector cycles through all three forms and the visible
    /// field list follows it.
    #[test]
    fn sandbox_kind_cycles_and_visibility_follows() {
        let mut a = full_adapter();
        assert_eq!(sandbox_kind(&a), "args");
        let with_args = visible_fields(&a).len();

        cycle_sandbox(&mut a);
        assert_eq!(sandbox_kind(&a), "none");
        assert_eq!(visible_fields(&a).len(), with_args - 2);

        cycle_sandbox(&mut a);
        assert_eq!(sandbox_kind(&a), "flag");
        assert_eq!(visible_fields(&a).len(), with_args + 1);

        cycle_sandbox(&mut a);
        assert_eq!(sandbox_kind(&a), "args");
    }

    /// Flag-form setters must not touch an args-form sandbox (and vice versa) —
    /// they only apply when their form is active.
    #[test]
    fn sandbox_setters_respect_active_form() {
        let mut a = full_adapter(); // args form
        let flag_field = SANDBOX_FLAG_FIELDS
            .iter()
            .find(|f| f.label == "sandbox flag")
            .unwrap();
        let before = a.sandbox.clone();
        (flag_field.set)(&mut a, "--permission-mode");
        assert_eq!(a.sandbox, before, "flag setter must not alter args form");
    }

    /// Bool fields parse only the literal `true`.
    #[test]
    fn bool_fields_toggle() {
        let mut a = full_adapter();
        let stream = IDENTITY_AND_CORE
            .iter()
            .find(|f| f.label == "stream")
            .unwrap();
        assert_eq!((stream.get)(&a), "false");
        (stream.set)(&mut a, "true");
        assert!(a.capabilities.stream);
        (stream.set)(&mut a, "false");
        assert!(!a.capabilities.stream);
    }
}
