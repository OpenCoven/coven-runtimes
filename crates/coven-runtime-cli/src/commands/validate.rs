//! `conjure validate` — check a manifest against the shared spec rules.

use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Args;
use coven_runtime_spec::validate_manifest;

use super::{load_manifest, load_registry};

#[derive(Args)]
pub struct ValidateArgs {
    /// Path to the adapter manifest JSON (or registry index, with --registry).
    pub manifest: PathBuf,
    /// Treat the input as a registry index (`{ "runtimes": { ... } }`) rather
    /// than an adapter manifest, and validate every entry plus id/key match.
    #[arg(long)]
    pub registry: bool,
    /// Print the parsed adapter summary even when valid.
    #[arg(long)]
    pub verbose: bool,
}

pub fn run(args: ValidateArgs) -> Result<()> {
    if args.registry {
        return run_registry(&args);
    }
    let manifest = load_manifest(&args.manifest)?;
    let errors = validate_manifest(&manifest);

    if args.verbose {
        for adapter in &manifest.adapters {
            let caps: Vec<&str> = adapter
                .capabilities
                .as_pairs()
                .iter()
                .filter(|(_, on)| *on)
                .map(|(n, _)| *n)
                .collect();
            let caps = if caps.is_empty() {
                "baseline".to_string()
            } else {
                caps.join(", ")
            };
            println!(
                "· {} ({}) — exe `{}`, capabilities: {}",
                adapter.id, adapter.label, adapter.executable, caps
            );
        }
    }

    if errors.is_empty() {
        println!(
            "✓ {} valid ({} adapter{}).",
            args.manifest.display(),
            manifest.adapters.len(),
            if manifest.adapters.len() == 1 {
                ""
            } else {
                "s"
            }
        );
        Ok(())
    } else {
        for e in &errors {
            eprintln!("✗ {e}");
        }
        bail!("{} problem(s) found", errors.len());
    }
}

/// Validate a registry index: every adapter must pass the shared spec rules and
/// each entry's `adapter.id` must match its runtime key.
fn run_registry(args: &ValidateArgs) -> Result<()> {
    let index = load_registry(&args.manifest)?;
    let errors = index.validate();

    let runtime_count = index.runtimes.len();
    let entry_count: usize = index.runtimes.values().map(Vec::len).sum();

    if args.verbose {
        for (runtime_id, entries) in &index.runtimes {
            let versions: Vec<&str> = entries.iter().map(|e| e.version.as_str()).collect();
            println!("· {} — versions: {}", runtime_id, versions.join(", "));
        }
    }

    if errors.is_empty() {
        println!(
            "✓ {} valid ({} runtime{}, {} entr{}).",
            args.manifest.display(),
            runtime_count,
            if runtime_count == 1 { "" } else { "s" },
            entry_count,
            if entry_count == 1 { "y" } else { "ies" }
        );
        Ok(())
    } else {
        for e in &errors {
            eprintln!("✗ {e}");
        }
        bail!("{} problem(s) found", errors.len());
    }
}
