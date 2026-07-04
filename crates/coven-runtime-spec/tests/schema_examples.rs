//! Drift guard: the shipped JSON Schema (`schema/adapter-manifest.schema.json`)
//! must accept the example manifests **and** anything a valid [`RuntimeAdapter`]
//! serializes to. If the Rust types and the schema diverge, this fails.
//!
//! The schema is a repo-root artifact contributors point their editors/CI at, so
//! we validate the real file rather than a proxy.

use std::path::PathBuf;

use coven_runtime_spec::{
    AdapterManifest, Capabilities, RuntimeAdapter, SandboxMapping, StreamArgs,
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
    for name in ["examples/hermes.json", "examples/claude.json"] {
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
        sandbox: Some(SandboxMapping {
            flag: "--permission-mode".into(),
            full: "bypassPermissions".into(),
            read_only: "plan".into(),
        }),
        stream_args: Some(StreamArgs {
            prefix_args: vec!["-p".into(), "stream-json".into()],
            session_id_flag: Some("--session-id".into()),
            resume_flag: Some("--resume".into()),
        }),
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

/// A `covenrt new --flavor minimal` scaffold (baseline, no additions) must also
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
            version: None,
            homepage: None,
            description: None,
        }],
    };
    let instance = serde_json::to_value(&manifest).unwrap();
    assert_valid(&validator, &instance, "baseline adapter");
}
