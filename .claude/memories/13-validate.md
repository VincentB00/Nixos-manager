# Validate (check without building)

Added 2026-10-15. Toolbar "Validate" button and "Validate only" in the build
dialog; runs as the user, no sudo, nothing written to the system.

`crates/core/src/validate.rs::run(job, workdir, host)`:
1. `nix-instantiate --parse` on every .nix file → syntax errors listed with
   file names; stop if any.
2. Stage a temp copy (0700) of the working folder excluding .git, .claude,
   node_modules, result, target and secrets.age; borrow flake.lock and
   hardware-configuration.nix from /etc/nixos when the working copy lacks
   them (mirrors sync.sh), and say so in the log.
3. Flake: `nix build --dry-run --no-write-lock-file --no-link
   path:<tmp>#nixosConfigurations.<host>.config.system.build.toplevel`.
   Classic: `NIXOS_CONFIG=<tmp>/configuration.nix nixos-rebuild dry-build`.
   Exit 0 → succeeded ("evaluates and can be built"), else failed with the
   nix error in the job log. Temp dir removed afterwards.
Route `POST /api/validate {path, host}` → JobInfo; UI navigates to the job log.
Note: `path:` flakes copy the folder into the store, hence secrets.age is
excluded from the staged copy (it is encrypted anyway, and normal
`--flake /etc/nixos` builds copy it too).
