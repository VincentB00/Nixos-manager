# Classic nix-shell environment with everything needed to build and run
# Nixos-manager. Used by ./run.sh; `nix develop` (flake.nix) is the alternative.
{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  packages = with pkgs; [
    # Rust daemon
    cargo
    rustc
    rustfmt
    clippy
    pkg-config
    # Angular UI
    nodejs_24
    # ./run.sh --watch
    watchexec
    # Nice to have
    nvd
    nix-output-monitor
  ];
  # Deliberately NOT adding nix, nixos-rebuild or sudo: the daemon must use the
  # host's own (nixpkgs' sudo is not setuid and would break the build job).

  RUST_LOG = "info";
}
