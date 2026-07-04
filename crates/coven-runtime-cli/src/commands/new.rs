//! `conjure new` — scaffold a new adapter manifest.

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Args;
use coven_runtime_spec::validate::valid_adapter_id as valid_id;
use coven_runtime_spec::validate_manifest;

use crate::template::{scaffold, Flavor};

#[derive(Args)]
pub struct NewArgs {
    /// Canonical adapter id (lowercase letters, digits, '.', '_', '-').
    pub id: String,
    /// Starting point: `minimal` (one-shot) or `streaming` (stream + sandbox).
    #[arg(long, default_value = "minimal")]
    pub flavor: String,
    /// Output path. Defaults to `<id>.json` in the current directory.
    #[arg(long, short)]
    pub out: Option<PathBuf>,
    /// Overwrite the output file if it already exists.
    #[arg(long)]
    pub force: bool,
}

pub fn run(args: NewArgs) -> Result<()> {
    let id = args.id.trim().to_lowercase();
    if !valid_id(&id) {
        bail!(
            "invalid adapter id `{}`; use lowercase letters, digits, '.', '_' or '-'",
            args.id
        );
    }
    let flavor = Flavor::parse(&args.flavor).map_err(|e| anyhow::anyhow!(e))?;
    let manifest = scaffold(&id, flavor);

    // Scaffolds must always be valid — guard against template regressions.
    let errors = validate_manifest(&manifest);
    if !errors.is_empty() {
        bail!(
            "internal: scaffold produced an invalid manifest:\n{}",
            errors
                .iter()
                .map(|e| format!("  - {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    let out = args
        .out
        .unwrap_or_else(|| PathBuf::from(format!("{id}.json")));
    if out.exists() && !args.force {
        bail!(
            "{} already exists; pass --force to overwrite",
            out.display()
        );
    }
    let json = manifest.to_json_pretty().context("serialize scaffold")?;
    fs::write(&out, format!("{json}\n"))
        .with_context(|| format!("failed to write {}", out.display()))?;

    println!("Created {} ({} flavor).", out.display(), args.flavor);
    println!("Next: edit it, then `conjure validate {}`.", out.display());
    Ok(())
}
