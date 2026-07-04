//! covenrt — author, validate, conformance-test, and package Coven runtime adapters.

mod commands;
mod sha256;
mod template;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "covenrt",
    version,
    about = "Author and validate Coven runtime adapters",
    long_about = "covenrt is the authoring toolkit for the Coven runtime SDK.\n\
                  Scaffold a new adapter, validate a manifest against the shared \
                  spec, run conformance checks against the runtime binary, and \
                  package a manifest for publishing to a registry."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold a new adapter manifest from a template.
    New(commands::new::NewArgs),
    /// Validate an adapter manifest against the shared spec rules.
    Validate(commands::validate::ValidateArgs),
    /// Run conformance checks: probe the runtime binary for its declared capabilities.
    Test(commands::test::TestArgs),
    /// Package a validated manifest for publishing (canonical JSON + checksum).
    Package(commands::package::PackageArgs),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::New(args) => commands::new::run(args),
        Command::Validate(args) => commands::validate::run(args),
        Command::Test(args) => commands::test::run(args),
        Command::Package(args) => commands::package::run(args),
    }
}
