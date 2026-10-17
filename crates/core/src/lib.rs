//! Core library for Nixos-manager.
//!
//! Everything here is UI-agnostic. The daemon crate exposes it over HTTP, the
//! cli crate exposes it on the command line. Modules:
//!
//! - [`types`]    shared API types (exported to TypeScript with ts-rs)
//! - [`state`]    the small JSON state file (recent locations, preferences)
//! - [`fsbrowse`] server-side directory browsing and location suggestions
//! - [`fsops`]    create/rename/duplicate/delete files and folders in a config
//! - [`nixparse`] rnix-based analysis of one .nix file into a generic node tree
//! - [`discover`] whole-config discovery: file classification, import graph
//! - [`edit`]     lossless text edits on .nix files driven by node ids
//! - [`nix`]      wrappers around the system `nix` / `nixos-*` commands
//! - [`jobs`]     long-running job registry with log streaming
//! - [`options`]  NixOS option documentation index (options.json)
//! - [`packages`] package search via `nix search`
//! - [`catalog`]  curated wizard recipes rendered into [`types::ConfigChange`]s
//! - [`deploy`]   sync-to-/etc/nixos (sync.sh semantics) plus nixos-rebuild

pub mod catalog;
pub mod channels;
pub mod deploy;
pub mod discover;
pub mod disks;
pub mod edit;
pub mod error;
pub mod flake;
pub mod fsbrowse;
pub mod fsops;
pub mod git;
pub mod hardware;
pub mod jobs;
pub mod nix;
pub mod nixparse;
pub mod options;
pub mod organize;
pub mod packages;
pub mod process;
pub mod remote;
pub mod search;
pub mod secrets;
pub mod state;
pub mod system;
pub mod types;
pub mod validate;

pub use error::{CoreError, Result};
