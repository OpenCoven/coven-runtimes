//! Sandbox / permission mapping for a runtime.
//!
//! Mirrors `coven`'s `SandboxMapping` (`harness.rs`), which maps the composer's
//! Access chip (`full` / `read-only`) to a runtime's native sandbox flag —
//! Codex `--sandbox danger-full-access|read-only`, Claude
//! `--permission-mode bypassPermissions|plan`.
//!
//! Today external adapters can't declare this at all: coven's
//! `ExternalHarnessAdapterSpec::into_spec` hardcodes `sandbox: None`, so
//! `coven run --permission` is a warned no-op for every manifest-based runtime.
//! Declaring it here closes that gap.

use serde::{Deserialize, Serialize};

/// How a runtime translates a permission policy into its native sandbox flag.
///
/// `flag` is the CLI flag name (e.g. `--sandbox`); `full` / `read_only` are the
/// values passed for each policy. Naming follows the manifest convention:
/// snake_case canonical with camelCase/kebab aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxMapping {
    /// CLI flag name, e.g. `"--sandbox"` (Codex) or `"--permission-mode"` (Claude).
    pub flag: String,
    /// Value passed for the `Full` (unrestricted) policy,
    /// e.g. `"danger-full-access"`.
    pub full: String,
    /// Value passed for the `ReadOnly` policy, e.g. `"read-only"` or `"plan"`.
    /// Canonical name is snake_case `read_only`; aliases accept camelCase and
    /// kebab-case spellings too.
    #[serde(alias = "readOnly", alias = "read-only")]
    pub read_only: String,
}

impl SandboxMapping {
    /// Argv tokens for a permission policy: `[flag, value]`.
    pub fn args(&self, permission: Permission) -> [String; 2] {
        [
            self.flag.clone(),
            match permission {
                Permission::Full => self.full.clone(),
                Permission::ReadOnly => self.read_only.clone(),
            },
        ]
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
        SandboxMapping {
            flag: "--sandbox".into(),
            full: "danger-full-access".into(),
            read_only: "read-only".into(),
        }
    }

    #[test]
    fn args_map_each_policy() {
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
    fn read_only_accepts_camel_and_kebab_aliases() {
        let camel: SandboxMapping = serde_json::from_str(
            r#"{ "flag": "--permission-mode", "full": "bypassPermissions", "readOnly": "plan" }"#,
        )
        .unwrap();
        assert_eq!(camel.read_only, "plan");

        let kebab: SandboxMapping = serde_json::from_str(
            r#"{ "flag": "--permission-mode", "full": "bypassPermissions", "read-only": "plan" }"#,
        )
        .unwrap();
        assert_eq!(kebab.read_only, "plan");

        let snake: SandboxMapping = serde_json::from_str(
            r#"{ "flag": "--permission-mode", "full": "bypassPermissions", "read_only": "plan" }"#,
        )
        .unwrap();
        assert_eq!(snake.read_only, "plan");
    }

    #[test]
    fn permission_round_trips_kebab() {
        assert_eq!(Permission::ReadOnly.as_str(), "read-only");
        let p: Permission = serde_json::from_str(r#""read-only""#).unwrap();
        assert_eq!(p, Permission::ReadOnly);
    }
}
