//! Drift guard: the shipped JSON Schema (`schema/adapter-manifest.schema.json`)
//! must accept the example manifests **and** anything a valid [`RuntimeAdapter`]
//! serializes to. If the Rust types and the schema diverge, this fails.
//!
//! The schema is a repo-root artifact contributors point their editors/CI at, so
//! we validate the real file rather than a proxy.

use std::path::PathBuf;

use coven_runtime_spec::{
    AdapterManifest, Capabilities, ContinuityArgs, RuntimeAdapter, SandboxMapping, StreamArgs,
};
use serde_json::Value;

/// Repo root = two levels up from this crate's manifest dir
/// (`crates/coven-runtime-spec` → repo root).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crate is nested two levels under the repo root")
        .to_path_buf()
}

fn load_json(rel: &str) -> Value {
    let path = repo_root().join(rel);
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn manifest_schema() -> jsonschema::Validator {
    let schema = load_json("schema/adapter-manifest.schema.json");
    jsonschema::validator_for(&schema).expect("adapter-manifest schema compiles")
}

fn assert_valid(validator: &jsonschema::Validator, instance: &Value, label: &str) {
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| e.to_string())
        .collect();
    assert!(
        errors.is_empty(),
        "{label} failed schema:\n  {}",
        errors.join("\n  ")
    );
}

#[test]
fn schema_accepts_example_manifests() {
    let validator = manifest_schema();
    for name in [
        "examples/hermes.json",
        "examples/claude.json",
        "examples/copilot.json",
        "examples/opencode.json",
        "examples/grok.json",
    ] {
        let instance = load_json(name);
        assert_valid(&validator, &instance, name);
    }
}

#[test]
fn schema_rejects_unknown_fields() {
    // additionalProperties:false must actually catch typos.
    let validator = manifest_schema();
    let bad: Value = serde_json::json!({
        "adapters": [{
            "id": "x", "label": "X", "executable": "x",
            "install_hint": "install",
            "capabilties": { "stream": true }  // deliberate typo
        }]
    });
    assert!(
        validator.iter_errors(&bad).next().is_some(),
        "schema should reject an unknown `capabilties` field"
    );
}

#[test]
fn schema_rejects_bad_id() {
    let validator = manifest_schema();
    let bad: Value = serde_json::json!({
        "adapters": [{
            "id": "Bad Id!", "label": "X", "executable": "x", "install_hint": "install"
        }]
    });
    assert!(
        validator.iter_errors(&bad).next().is_some(),
        "schema should reject an id that violates the charset pattern"
    );
}

#[test]
fn schema_rejects_blank_prompt_and_continuity_resume_flags() {
    let validator = manifest_schema();

    for field in [
        "prompt_flag",
        "promptFlag",
        "interactive_prompt_flag",
        "interactivePromptFlag",
    ] {
        let mut adapter = serde_json::json!({
            "id": "x", "label": "X", "executable": "x", "install_hint": "install"
        });
        adapter[field] = Value::String("   ".into());
        let manifest = serde_json::json!({ "adapters": [adapter] });
        assert!(
            validator.iter_errors(&manifest).next().is_some(),
            "schema should reject whitespace-only `{field}`"
        );
    }

    for field in ["resume_flag", "resumeFlag"] {
        let mut continuity = serde_json::json!({ "init_prefix_args": ["run"] });
        continuity[field] = Value::String("   ".into());
        let manifest = serde_json::json!({
            "adapters": [{
                "id": "x", "label": "X", "executable": "x", "install_hint": "install",
                "continuity_args": continuity
            }]
        });
        assert!(
            validator.iter_errors(&manifest).next().is_some(),
            "schema should reject whitespace-only continuity `{field}`"
        );
    }
}

/// The most important direction: anything the Rust type produces must satisfy
/// the schema. A fully-populated streaming adapter exercises every added block.
#[test]
fn schema_accepts_serialized_runtime_adapter() {
    let validator = manifest_schema();
    let adapter = RuntimeAdapter {
        id: "aria".into(),
        label: "Aria".into(),
        executable: "aria".into(),
        interactive_prompt_prefix_args: vec![],
        non_interactive_prompt_prefix_args: vec!["exec".into()],
        install_hint: "Install aria and add it to PATH.".into(),
        system_prompt_flag: Some("--system-prompt".into()),
        model_flag: Some("--model".into()),
        model_arg_template: None,
        capabilities: Capabilities {
            stream: true,
            preassigned_session_id: true,
            think: true,
            speed: true,
        },
        sandbox: Some(SandboxMapping::Flag {
            flag: "--permission-mode".into(),
            full: "bypassPermissions".into(),
            read_only: "plan".into(),
        }),
        stream_args: Some(StreamArgs {
            prefix_args: vec!["-p".into(), "stream-json".into()],
            session_id_flag: Some("--session-id".into()),
            resume_flag: Some("--resume".into()),
        }),
        prompt_flag: None,
        interactive_prompt_flag: None,
        continuity_args: None,
        version: Some("1.0.0".into()),
        homepage: Some("https://example.com".into()),
        description: Some("An example runtime.".into()),
    };
    let manifest = AdapterManifest {
        adapters: vec![adapter],
    };
    let instance = serde_json::to_value(&manifest).unwrap();
    assert_valid(&validator, &instance, "serialized RuntimeAdapter");
}

/// The args-form sandbox (per-policy argv lists, used by the Copilot adapter)
/// must also survive serialization → schema validation.
#[test]
fn schema_accepts_args_form_sandbox_adapter() {
    let validator = manifest_schema();
    let adapter = RuntimeAdapter {
        id: "copilot-like".into(),
        label: "Copilot-like".into(),
        executable: "copilot".into(),
        interactive_prompt_prefix_args: vec!["-i".into()],
        non_interactive_prompt_prefix_args: vec!["-s".into(), "-p".into()],
        install_hint: "Install the runtime and add it to PATH.".into(),
        system_prompt_flag: None,
        model_flag: Some("--model".into()),
        model_arg_template: None,
        capabilities: Capabilities {
            stream: true,
            preassigned_session_id: true,
            think: false,
            speed: false,
        },
        sandbox: Some(SandboxMapping::Args {
            full_args: vec!["--allow-all".into()],
            read_only_args: vec![
                "--deny-tool".into(),
                "write".into(),
                "--deny-tool".into(),
                "shell".into(),
            ],
        }),
        stream_args: Some(StreamArgs {
            prefix_args: vec![
                "--output-format".into(),
                "json".into(),
                "--stream".into(),
                "on".into(),
                "-p".into(),
            ],
            session_id_flag: Some("--session-id".into()),
            resume_flag: Some("--resume".into()),
        }),
        prompt_flag: None,
        interactive_prompt_flag: None,
        continuity_args: None,
        version: Some("1.0.0".into()),
        homepage: None,
        description: None,
    };
    let manifest = AdapterManifest {
        adapters: vec![adapter],
    };
    let instance = serde_json::to_value(&manifest).unwrap();
    assert_valid(&validator, &instance, "args-form sandbox adapter");
}

/// A one-shot plain-mode headless adapter (Grok Build shape: flag-bound
/// prompt, continuity args, no stream mode) must survive serialization →
/// schema validation, exercising every field added for one-shot headless
/// runtimes.
#[test]
fn schema_accepts_one_shot_headless_adapter() {
    let validator = manifest_schema();
    let adapter = RuntimeAdapter {
        id: "grok".into(),
        label: "Grok Build".into(),
        executable: "grok".into(),
        interactive_prompt_prefix_args: vec![
            "--no-auto-update".into(),
            "--no-alt-screen".into(),
            "--output-format".into(),
            "plain".into(),
        ],
        non_interactive_prompt_prefix_args: vec![
            "--no-auto-update".into(),
            "--no-alt-screen".into(),
            "--output-format".into(),
            "plain".into(),
        ],
        prompt_flag: Some("--single".into()),
        interactive_prompt_flag: Some("--single".into()),
        install_hint: "Install Grok Build and run `grok login`.".into(),
        system_prompt_flag: Some("--rules".into()),
        model_flag: Some("--model".into()),
        model_arg_template: None,
        capabilities: Capabilities {
            stream: false,
            preassigned_session_id: true,
            think: false,
            speed: false,
        },
        sandbox: Some(SandboxMapping::Args {
            full_args: vec![
                "--permission-mode".into(),
                "bypassPermissions".into(),
                "--sandbox".into(),
                "off".into(),
            ],
            read_only_args: vec![
                "--permission-mode".into(),
                "default".into(),
                "--sandbox".into(),
                "read-only".into(),
            ],
        }),
        stream_args: None,
        continuity_args: Some(ContinuityArgs {
            init_prefix_args: vec![
                "--no-auto-update".into(),
                "--no-alt-screen".into(),
                "--output-format".into(),
                "plain".into(),
            ],
            resume_prefix_args: vec![
                "--no-auto-update".into(),
                "--no-alt-screen".into(),
                "--output-format".into(),
                "plain".into(),
            ],
            session_id_flag: Some("--session-id".into()),
            resume_flag: Some("--resume".into()),
        }),
        version: Some("1.0.0".into()),
        homepage: Some("https://docs.x.ai/build/cli/headless-scripting".into()),
        description: Some("Grok Build headless runtime adapter.".into()),
    };
    let manifest = AdapterManifest {
        adapters: vec![adapter],
    };
    let instance = serde_json::to_value(&manifest).unwrap();
    assert_valid(&validator, &instance, "one-shot headless adapter");
}

/// A `conjure new --flavor minimal` scaffold (baseline, no additions) must also
/// satisfy the schema — the common contributor starting point.
#[test]
fn schema_accepts_baseline_adapter() {
    let validator = manifest_schema();
    let manifest = AdapterManifest {
        adapters: vec![RuntimeAdapter {
            id: "minimal".into(),
            label: "Minimal".into(),
            executable: "minimal".into(),
            interactive_prompt_prefix_args: vec![],
            non_interactive_prompt_prefix_args: vec!["exec".into()],
            install_hint: "install".into(),
            system_prompt_flag: None,
            model_flag: None,
            model_arg_template: None,
            capabilities: Capabilities::BASELINE,
            sandbox: None,
            stream_args: None,
            prompt_flag: None,
            interactive_prompt_flag: None,
            continuity_args: None,
            version: None,
            homepage: None,
            description: None,
        }],
    };
    let instance = serde_json::to_value(&manifest).unwrap();
    assert_valid(&validator, &instance, "baseline adapter");
}
