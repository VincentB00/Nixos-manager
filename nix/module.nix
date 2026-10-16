# NixOS module: runs the Nixos-manager daemon as a *user* service so it can
# read and write the configuration folder in the user's home. Privileged
# steps (sync to /etc/nixos, nixos-rebuild) prompt for the sudo password in
# the UI; the daemon itself never runs as root.
{ self }:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.nixos-manager;
  pkg = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
in
{
  options.services.nixos-manager = {
    enable = lib.mkEnableOption "Nixos-manager web GUI (user service)";
    package = lib.mkOption {
      type = lib.types.package;
      default = pkg;
      description = "Nixos-manager package to run.";
    };
    listen = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:7788";
      description = ''
        Address the daemon listens on. Keep it on localhost unless you put TLS
        in front: the sudo password travels over this connection.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];
    systemd.user.services.nixos-manager = {
      description = "Nixos-manager web GUI";
      wantedBy = [ "default.target" ];
      after = [ "network.target" ];
      path = [ pkgs.nix pkgs.nixos-rebuild pkgs.sudo pkgs.bash pkgs.coreutils pkgs.diffutils pkgs.hostname pkgs.git ];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/nixos-manager-daemon --listen ${cfg.listen}";
        Restart = "on-failure";
        RestartSec = 2;
      };
    };
  };
}
