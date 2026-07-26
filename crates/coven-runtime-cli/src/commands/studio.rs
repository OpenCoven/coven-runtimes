//! `conjure studio` — interactive TUI for authoring adapter manifests.

use std::io::{stdin, stdout, IsTerminal};
use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Args;
use coven_runtime_spec::validate::valid_adapter_id as valid_id;

use crate::commands::load_manifest;
use crate::studio;
use crate::template::{scaffold, Flavor};

#[derive(Args)]
pub struct StudioArgs {
    /// Manifest to edit. If it doesn't exist yet, the studio opens a scaffold
    /// named after the file stem and creates the file on first save.
    pub manifest: PathBuf,
    /// Scaffold flavor when the manifest doesn't exist yet: `minimal`,
    /// `streaming`, or `continuity`.
    #[arg(long, default_value = "minimal")]
    pub flavor: String,
}

pub fn run(args: StudioArgs) -> Result<()> {
    if !stdin().is_terminal() || !stdout().is_terminal() {
        bail!(
            "conjure studio requires an interactive terminal; \
             use `conjure new`/`conjure validate` in scripts"
        );
    }

    let (manifest, fresh) = if args.manifest.exists() {
        (load_manifest(&args.manifest)?, false)
    } else {
        let id = args
            .manifest
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.trim().to_lowercase())
            .unwrap_or_default();
        if !valid_id(&id) {
            bail!(
                "cannot derive an adapter id from `{}`; name the file after the id \
                 (lowercase letters, digits, '.', '_' or '-'), e.g. `conjure studio aria.json`",
                args.manifest.display()
            );
        }
        let flavor = Flavor::parse(&args.flavor).map_err(|e| anyhow::anyhow!(e))?;
        (scaffold(&id, flavor), true)
    };

    if manifest.adapters.is_empty() {
        bail!(
            "{} declares no adapters; add one before opening the studio",
            args.manifest.display()
        );
    }

    studio::run(manifest, args.manifest, fresh)
}
