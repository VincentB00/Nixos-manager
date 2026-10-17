//! Dev helper: prints the generated remote deploy / trust scripts so they can be run against a fake target.
use nixos_manager_core::remote::{remote_grant_script, remote_revoke_script, remote_script, REMOTE_IS_TRUSTED_FN};
use nixos_manager_core::types::{ConfigKind, DeployRequest, DeployTarget, RebuildAction};
use std::path::Path;
fn main() {
    let which = std::env::args().nth(1).unwrap_or_default();
    match which.as_str() {
        "grant" => print!("{}", remote_grant_script("tester")),
        "revoke" => print!("{}", remote_revoke_script("tester")),
        "fns" => print!("{}", REMOTE_IS_TRUSTED_FN),
        "deploy" | "deploy-reboot" | "deploy-reboot-always" => {
            let req = DeployRequest { path: "/w".into(), host: Some("main".into()), action: RebuildAction::Switch, upgrade: false, keep_configuration: None, hardware: None, skip_sync: false, password: String::new(), secrets_password: None, target: None, target_sudo_password: None, build_host: None, after: if which.starts_with("deploy-reboot") { Some(nixos_manager_core::types::AfterBuild::Reboot) } else { None }, after_even_on_failure: which == "deploy-reboot-always", after_local: None, on_target: false, allow_unfree_env: false };
            let t = DeployTarget { name: "fake".into(), ssh_target: "tester@fakehost".into(), flake_host: Some("main".into()), build_host: None, use_remote_sudo: true, port: None };
            print!("{}", remote_script(&req, &t, Path::new(&std::env::args().nth(2).unwrap_or("/tmp".into())), ConfigKind::Flake, "--sudo", Path::new("/tmp/nm-stop-test")));
        }
        "builder" => {
            // the build-host fragment of the local root script + the rebuild line, runnable under fakes
            let t = DeployTarget { name: "fake".into(), ssh_target: "tester@fakehost".into(), flake_host: None, build_host: None, use_remote_sudo: true, port: None };
            let mut s = String::from("set -euo pipefail\nNIXOS_DIR=/tmp\n");
            s.push_str(&nixos_manager_core::deploy::build_host_block(&t, std::path::Path::new("/nonexistent")));
            s.push_str(&nixos_manager_core::deploy::stoppable_pub("nixos-rebuild switch --flake \"$NIXOS_DIR#main\" --build-host 'tester@fakehost' 2>&1", Path::new("/tmp/nm-stop-test")));
            s.push_str(&nixos_manager_core::remote::trust_snippet_pub(&t, false));
            s.push_str("if [ \"$RC\" -ne 0 ]; then exit \"$RC\"; fi\necho \"[nixos-manager] done\"\n");
            print!("{s}");
        }
        "on-target" => {
            // wrapper + the uploaded root script, for the fake-target e2e
            let staged = std::env::args().nth(2).unwrap_or("/tmp/staged".into());
            let t = DeployTarget { name: "fake".into(), ssh_target: "tester@fakehost".into(), flake_host: Some("main".into()), build_host: None, use_remote_sudo: true, port: None };
            let req = DeployRequest { path: "/w".into(), host: Some("main".into()), action: RebuildAction::Switch, upgrade: false, keep_configuration: Some(nixos_manager_core::types::KeepConfiguration::Local), hardware: None, skip_sync: false, password: String::new(), secrets_password: None, target: None, target_sudo_password: None, build_host: None, after: None, after_even_on_failure: false, after_local: None, on_target: true, allow_unfree_env: true };
            let env = nixos_manager_core::deploy::ScriptEnv { exports: vec![("NIX_CONFIG".into(), "experimental-features = nix-command flakes".into()), ("NIXPKGS_ALLOW_UNFREE".into(), "1".into())], impure: true };
            let dir = std::env::args().nth(3).unwrap_or("/tmp/nixos-manager-deploy-test".into());
            let script = nixos_manager_core::deploy::build_script_with_pub(&req, Path::new(&dir), ConfigKind::Flake, 0, 0, Path::new(&format!("{dir}.stop")), None, &env);
            std::fs::write(Path::new(&staged).join(".nixos-manager-deploy.sh"), script).unwrap();
            print!("{}", nixos_manager_core::remote::on_target_wrapper(&t, Path::new(&staged), &dir, Path::new("/tmp/nm-stop-test")));
        }
        _ => eprintln!("usage: dump_scripts grant|revoke|fns|deploy <staged>|builder|on-target <staged> <remote dir>"),
    }
}
