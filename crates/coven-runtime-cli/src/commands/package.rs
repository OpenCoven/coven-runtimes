//! `covenrt package` — emit canonical JSON + checksum for publishing.
//!
//! Validates the manifest, then writes a canonicalized copy (stable key order,
//! trailing newline) and prints its SHA-256. The checksum is what a registry
//! [`coven_runtime_registry::RegistryEntry`] records in its `sha256` field, so
//! consumers can verify integrity after fetch.

use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Args;
use coven_runtime_spec::validate_manifest;

use super::load_manifest;
use crate::sha256::sha256_hex;

#[derive(Args)]
pub struct PackageArgs {
    /// Path to the adapter manifest JSON.
    pub manifest: PathBuf,
    /// Write the canonicalized manifest here (default: `<name>.pkg.json`).
    #[arg(long, short)]
    pub out: Option<PathBuf>,
    /// Print the checksum only; don't write a canonical file.
    #[arg(long)]
    pub check_only: bool,
}

pub fn run(args: PackageArgs) -> Result<()> {
    let manifest = load_manifest(&args.manifest)?;

    let errors = validate_manifest(&manifest);
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("✗ {e}");
        }
        bail!(
            "cannot package an invalid manifest ({} problem(s))",
            errors.len()
        );
    }

    // Canonical form: pretty JSON from the parsed model (drops unknown fields,
    // normalizes ordering) + trailing newline.
    let canonical = format!("{}\n", manifest.to_json_pretty().context("serialize")?);
    let digest = sha256_hex(canonical.as_bytes());

    if args.check_only {
        println!("sha256:{digest}");
        return Ok(());
    }

    let out = args.out.unwrap_or_else(|| {
        let stem = args
            .manifest
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "adapter".to_string());
        PathBuf::from(format!("{stem}.pkg.json"))
    });
    fs::write(&out, &canonical).with_context(|| format!("failed to write {}", out.display()))?;

    println!("Packaged {} → {}", args.manifest.display(), out.display());
    println!("sha256:{digest}");
    Ok(())
}
