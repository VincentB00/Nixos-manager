# Managed secrets (secrets.age)

Added 2026-10-15 after a consult: Vincent kept SMB logins as plain files in
the config repo (world-readable, pushed to GitHub). Design agreed: one
passphrase-encrypted file instead of key-based sops-nix, so beginners have
nothing to generate. Users who do not create the file see no prompts.

## Model (`crates/core/src/secrets.rs`)
- `secrets.age` in the config folder: standard age passphrase file (scrypt +
  ChaCha20-Poly1305 via the `age` crate) containing JSON
  `{version, entries: {name: SecretEntry}}`. `age -d secrets.age` works
  without the app. Safe to commit; sync.sh copies it to /etc/nixos harmlessly.
- `SecretEntry { name, kind: smb|text, username, password, domain, content }`.
  smb renders as mount.cifs credentials format (`username=..\npassword=..`).
- `SecretsVault`: per-config-folder unlocked state in daemon memory
  (payload + password for re-encryption), auto-locks after 15 min idle.
  Status redacts passwords. Routes `/api/secrets/{status,create,unlock,lock,
  get,set,remove,change-password}`.
- Deploy: `DeployRequest.secrets_password` (optional). `stage_for_deploy`
  decrypts as the user into a 0700 temp dir; `install_script` (inserted into
  the root deploy script before `cd /etc/nixos`) installs files 600 root into
  `/var/lib/nixos-manager/secrets/<name>` (dir 755), removes entries no
  longer present, writes `.deployed-hash` (sha256 of secrets.age, 644) so
  `SecretsDeployInfo.deployed_up_to_date` can be computed without root.
  Preflight carries `secrets: {present, deployed_up_to_date, deploy_dir}`.
  No password given + changed file → job logs a note and keeps old files.
- Recipe `mount_smb`: new default auth `managed` with a `Secret` field kind
  → `credentials=/var/lib/nixos-manager/secrets/<name>`; validation requires
  a secret when managed is chosen.

## UI
- Sidebar "Secrets" (all configs). `pages/secrets/secrets.component.ts`:
  create (password + repeat), unlock, entries list with reveal/edit/remove,
  add (`secret-entry-dialog`), change password, lock; shows deployed state.
  Each entry shows its installed path and a copy menu (plain path, or the
  ready-made `"credentials=<path>"` mount option); clipboard API with an
  execCommand fallback for non-secure contexts.
- Build dialog: secrets password field when secrets.age exists (optional
  when up to date, recommended when changed).
- Wizard `secret` field: inline unlock, then a dropdown of entry names.

## Caveats (documented to Vincent)
Plaintext must exist on disk for CIFS mounts (root-only, like sops-nix);
rebuilds outside the app keep the last installed files; the password travels
over the local HTTP connection like the sudo password.
