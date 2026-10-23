{
  description = "Nixos-manager: a web GUI for NixOS configurations (Rust daemon + Angular UI)";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        version = "0.1.0";

        # Angular bundle. Rebuild the hash after changing ui/package-lock.json:
        #   nix run nixpkgs#prefetch-npm-deps -- ui/package-lock.json
        ui = pkgs.buildNpmPackage {
          pname = "nixos-manager-ui";
          inherit version;
          src = ./ui;
          npmDepsHash = "sha256-s8TxhRGrs6bdt6qJ1apM3JiF3guFudZwOxjyWgMPa20=";
          npmBuildScript = "build";
          installPhase = ''
            mkdir -p $out
            cp -r dist/nixos-manager-ui/browser/* $out/
          '';
        };

        daemon = pkgs.rustPlatform.buildRustPackage {
          pname = "nixos-manager";
          inherit version;
          src = pkgs.lib.cleanSourceWith {
            src = ./.;
            filter = path: type:
              let rel = pkgs.lib.removePrefix (toString ./. + "/") (toString path);
              in !(pkgs.lib.hasPrefix "ui" rel || pkgs.lib.hasPrefix "target" rel || pkgs.lib.hasPrefix ".claude" rel);
          };
          cargoLock.lockFile = ./Cargo.lock;
          # Put the built UI where rust-embed expects it before compiling.
          preBuild = ''
            mkdir -p ui/dist/nixos-manager-ui/browser
            cp -r ${ui}/* ui/dist/nixos-manager-ui/browser/
          '';
          meta = with pkgs.lib; {
            description = "Web GUI for NixOS configurations";
            license = licenses.mit;
            mainProgram = "nixos-manager-daemon";
          };
        };
      in
      {
        packages = {
          default = daemon;
          daemon = daemon;
          ui = ui;
        };

        apps.default = {
          type = "app";
          program = "${daemon}/bin/nixos-manager-daemon";
        };
        # terminal UI for SSH sessions / headless machines: nix run .#tui -- [folder]
        apps.tui = {
          type = "app";
          program = "${daemon}/bin/nixos-manager-tui";
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            cargo rustc rustfmt clippy rust-analyzer
            nodejs_24
            nixfmt-rfc-style nil
            nvd nix-output-monitor
          ];
          RUST_LOG = "info";
          shellHook = ''
            echo "Nixos-manager dev shell"
            echo "  cargo run -p nixos-manager-daemon        # API on 127.0.0.1:7788"
            echo "  (cd ui && npm start)                     # UI on :4200, proxies /api"
            echo "  scripts/gen-types.sh                     # regenerate TS types from Rust"
          '';
        };
      })
    // {
      nixosModules.default = import ./nix/module.nix { inherit self; };
    };
}
