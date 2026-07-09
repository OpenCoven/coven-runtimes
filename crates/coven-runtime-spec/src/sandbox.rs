//! Sandbox / permission mapping for a runtime.
//!
//! Mirrors `coven`'s `SandboxMapping` (`harness.rs`), which maps the composer's
//! Access chip (`full` / `read-only`) to a runtime's native sandbox flag —
//! Codex `--sandbox danger-full-access|read-only`, Claude
//! `--permission-mode bypassPermissions|plan`.
//!
//! Not every runtime fits the `--flag value` shape, though. GitHub Copilot CLI
//! expresses permissions as boolean and repeatable flags (`--allow-all`,
//! `--deny-tool write --deny-tool shell`), so the mapping also supports a
//! per-policy argv-list form ([`SandboxMapping::Args`]).
//!
//! Today external adapters can't declare this at all: coven's
//! `ExternalHarnessAdapterSpec::into_spec` hardcodes `sandbox: None`, so
//! `coven run --permission` is a warned no-op for every manifest-based runtime.
//! Declaring it here closes that gap.

use serde::{Deserialize, Serialize};

/// How a runtime translates a permission policy into its native sandbox args.
///
/// Two shapes, distinguished structurally (untagged):
///
/// - [`SandboxMapping::Flag`] — one flag, one value per policy:
///   `{ "flag": "--sandbox", "full": "danger-full-access", "read_only": "read-only" }`
/// - [`SandboxMapping::Args`] — a full argv list per policy, for runtimes whose
///   permission flags are boolean or multi-token (GitHub Copilot CLI):
///   `{ "full_args": ["--allow-all"], "read_only_args": ["--deny-tool", "write", "--deny-tool", "shell"] }`
///
/// Naming follows the manifest convention: snake_case canonical with
/// camelCase (and, for `read_only`, kebab-case) aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum SandboxMapping {
    /// Single `--flag value` pair per policy (Codex, Claude).
    Flag {
        /// CLI flag name, e.g. `"--sandbox"` (Codex) or `"--permission-mode"` (Claude).
        flag: String,
        /// Value passed for the `Full` (unrestricted) policy,
        /// e.g. `"danger-full-access"`.
        full: String,
        /// Value passed for the `ReadOnly` policy, e.g. `"read-only"` or `"plan"`.
        /// Canonical name is snake_case `read_only`; aliases accept camelCase and
        /// kebab-case spellings too.
        #[serde(alias = "readOnly", alias = "read-only")]
        read_only: String,
    },
    /// A whole argv list per policy, for runtimes whose permission surface is
    /// boolean or multi-token flags (e.g. Copilot's `--allow-all` /
    /// `--deny-tool write --deny-tool shell`).
    Args {
        /// argv tokens appended for the `Full` (unrestricted) policy,
        /// e.g. `["--allow-all"]`.
        #[serde(alias = "fullArgs")]
        full_args: Vec<String>,
        /// argv tokens appended for the `ReadOnly` policy,
        /// e.g. `["--deny-tool", "write", "--deny-tool", "shell"]`.
        #[serde(alias = "readOnlyArgs")]
        read_only_args: Vec<String>,
    },
}

impl SandboxMapping {
    /// Argv tokens for a permission policy.
    pub fn args(&self, permission: Permission) -> Vec<String> {
        match self {
            SandboxMapping::Flag {
                flag,
                full,
                read_only,
            } => vec![
                flag.clone(),
                match permission {
                    Permission::Full => full.clone(),
                    Permission::ReadOnly => read_only.clone(),
                },
            ],
            SandboxMapping::Args {
                full_args,
                read_only_args,
            } => match permission {
                Permission::Full => full_args.clone(),
                Permission::ReadOnly => read_only_args.clone(),
            },
        }
    }

    /// The distinct flag-like tokens (`-…`) this mapping passes to the runtime,
    /// in declaration order. Used by `conjure test`'s soft probe warnings.
    pub fn probe_flags(&self) -> Vec<&str> {
        match self {
            SandboxMapping::Flag { flag, .. } => vec![flag.as_str()],
            SandboxMapping::Args {
                full_args,
                read_only_args,
            } => {
                let mut flags: Vec<&str> = Vec::new();
                for token in full_args.iter().chain(read_only_args.iter()) {
                    if token.starts_with('-') && !flags.contains(&token.as_str()) {
                        flags.push(token.as_str());
                    }
                }
                flags
            }
        }
    }
}

/// Sandbox/permission policy requested for a run. Mirrors `coven`'s `Permission`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    Full,
    ReadOnly,
}

impl Permission {
    /// Canonical kebab-case string form (`"full"` / `"read-only"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Permission::Full => "full",
            Permission::ReadOnly => "read-only",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codex_mapping() -> SandboxMapping {
        SandboxMapping::Flag {
            flag: "--sandbox".into(),
            full: "danger-full-access".into(),
            read_only: "read-only".into(),
        }
    }

    fn copilot_mapping() -> SandboxMapping {
        SandboxMapping::Args {
            full_args: vec!["--allow-all".into()],
            read_only_args: vec![
                "--deny-tool".into(),
                "write".into(),
                "--deny-tool".into(),
                "shell".into(),
            ],
        }
    }

    #[test]
    fn flag_form_maps_each_policy() {
        let m = codex_mapping();
        assert_eq!(
            m.args(Permission::Full),
            ["--sandbox".to_string(), "danger-full-access".to_string()]
        );
        assert_eq!(
            m.args(Permission::ReadOnly),
            ["--sandbox".to_string(), "read-only".to_string()]
        );
    }

    #[test]
    fn args_form_maps_each_policy() {
        let m = copilot_mapping();
        assert_eq!(m.args(Permission::Full), ["--allow-all".to_string()]);
        assert_eq!(
            m.args(Permission::ReadOnly),
            ["--deny-tool", "write", "--deny-tool", "shell"]
                .map(str::to_string)
                .to_vec()
        );
    }

    #[test]
    fn probe_flags_are_deduped_flag_tokens() {
        assert_eq!(codex_mapping().probe_flags(), ["--sandbox"]);
        // `--deny-tool` appears twice but is reported once; values are skipped.
        assert_eq!(
            copilot_mapping().probe_flags(),
            ["--allow-all", "--deny-tool"]
        );
    }

    #[test]
    fn flag_form_read_only_accepts_camel_and_kebab_aliases() {
        let camel: SandboxMapping = serde_json::from_str(
            r#"{ "flag": "--permission-mode", "full": "bypassPermissions", "readOnly": "plan" }"#,
        )
        .unwrap();
        let kebab: SandboxMapping = serde_json::from_str(
            r#"{ "flag": "--permission-mode", "full": "bypassPermissions", "read-only": "plan" }"#,
        )
        .unwrap();
        let snake: SandboxMapping = serde_json::from_str(
            r#"{ "flag": "--permission-mode", "full": "bypassPermissions", "read_only": "plan" }"#,
        )
        .unwrap();
        for m in [camel, kebab, snake] {
            match m {
                SandboxMapping::Flag { read_only, .. } => assert_eq!(read_only, "plan"),
                other => panic!("expected Flag form, got {other:?}"),
            }
        }
    }

    #[test]
    fn args_form_deserializes_snake_and_camel() {
        let snake: SandboxMapping = serde_json::from_str(
            r#"{ "full_args": ["--allow-all"], "read_only_args": ["--deny-tool", "write"] }"#,
        )
        .unwrap();
        let camel: SandboxMapping = serde_json::from_str(
            r#"{ "fullArgs": ["--allow-all"], "readOnlyArgs": ["--deny-tool", "write"] }"#,
        )
        .unwrap();
        assert_eq!(snake, camel);
        assert_eq!(snake.args(Permission::Full), ["--allow-all".to_string()]);
    }

    #[test]
    fn both_forms_round_trip_json() {
        for m in [codex_mapping(), copilot_mapping()] {
            let json = serde_json::to_string(&m).unwrap();
            let back: SandboxMapping = serde_json::from_str(&json).unwrap();
            assert_eq!(m, back);
        }
    }

    #[test]
    fn permission_round_trips_kebab() {
        assert_eq!(Permission::ReadOnly.as_str(), "read-only");
        let p: Permission = serde_json::from_str(r#""read-only""#).unwrap();
        assert_eq!(p, Permission::ReadOnly);
    }
}
