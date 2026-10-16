//! Command-line interface over the Nixos-manager core. Useful for scripting
//! and for checking what the daemon would see without starting it.

use clap::{Parser, Subcommand};
use nixos_manager_core::{discover, fsbrowse, nix};

#[derive(Parser)]
#[command(name = "nixos-manager", about = "Inspect a NixOS configuration the way the GUI does")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show what kind of configuration lives at PATH.
    Info { path: String },
    /// Full discovery: files, imports, namespaces, roots (JSON).
    Overview { path: String },
    /// Parse one file into the node tree the UI renders (JSON).
    File { path: String, file: String },
    /// Hosts from `nix flake show` (slow; evaluates the flake).
    Hosts { path: String },
    /// Likely configuration locations on this machine.
    Suggest,
    /// System generations.
    Generations,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let out = match cli.cmd {
        Cmd::Info { path } => serde_json::to_value(discover::location_info(&fsbrowse::normalize_input(&path))?)?,
        Cmd::Overview { path } => serde_json::to_value(discover::overview(&fsbrowse::normalize_input(&path)).await?)?,
        Cmd::File { path, file } => serde_json::to_value(discover::read_file_view(&fsbrowse::normalize_input(&path), &file)?)?,
        Cmd::Hosts { path } => serde_json::to_value(nix::flake_hosts(&fsbrowse::normalize_input(&path)).await?)?,
        Cmd::Suggest => serde_json::to_value(fsbrowse::suggestions())?,
        Cmd::Generations => serde_json::to_value(nix::generations().await?)?,
    };
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
