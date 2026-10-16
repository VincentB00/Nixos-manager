# "Add something" wizard (guided actions)

Added 2026-10-14 at Vincent's request: beginners who create a module do not
know what to do next, so the form view gets a guided "Add something" flow.

## UI (`ui/src/app/pages/wizard/wizard-dialog.component.ts`)
Opened from the file view's "Add something" button (form mode, .nix files
without syntax errors) and from the empty-module card. Three steps:
1. Pick: Install programs / Turn on a feature or service / Any NixOS option /
   Custom setting.
2. Details: package search with multi-select; recipe list grouped by
   category with a generated form; option search with official docs and a
   value field typed by the option's type (toggle for booleans, "append to
   list" checkbox for lists); custom key+value.
3. Preview: the Nix that will be added, warnings, post-apply notes. Apply.
Returns the new NixFileView to the file view, which reloads the overview.

## Backend
- `crates/core/src/catalog.rs`: 35 curated recipes (ssh, networkmanager,
  firewall, tailscale, avahi, syncthing, docker, podman, libvirt, plasma,
  gnome, autologin, sound, bluetooth, printing, flatpak, fonts, steam,
  hostname, timezone, locale, keyboard, user, allow_unfree, flakes, gc,
  auto_upgrade, zram, latest_kernel, systemd_boot, fwupd, zsh, git, nvidia,
  custom). Each has fields (bool/text/number/list/select) and templates with
  `{{field}}` (Nix literal) / `{{field:raw}}` placeholders; `when` guards;
  `AppendEach` expands comma lists. `render()` validates and returns
  `ConfigChange`s + preview + notes. Tests render every recipe.
- `crates/core/src/edit.rs` (bottom): `upsert_setting` (replace existing
  value found by walking nested attrsets, else add in the deepest existing
  parent), `append_to_list_setting` (dedup, honours `with pkgs;`),
  `add_package` (creates `with pkgs; [ ]` when missing; handles `++`),
  `ensure_module_arg` (adds `pkgs` to the module's pattern when needed),
  `apply_changes`. Rejects files with syntax errors.
- `crates/core/src/options.rs`: option docs index from nixpkgs' options.json
  (`nix-build -E '(import <nixpkgs/nixos> { configuration = {}; })
  .config.system.build.manual.optionsJSON'`). On NixOS it is already in the
  store (loads in ~1 s, 24.6k options); otherwise built as a job from the UI.
  Loaded lazily at daemon start. Search ranks name matches over description.
- `crates/core/src/packages.rs`: `nix search nixpkgs --json` wrapper, ~20 s
  on the first query (nix eval cache), <1 s after; memoised per query.
- Routes: `/api/options/{status,build,search,get}`, `/api/packages/search`,
  `/api/wizard/{recipes,render,apply}`. Apply checks the file version.

## Known limits / ideas
- Recipes are a fixed list in Rust; a JSON/Nix-defined catalog could let
  users add their own later.
- `custom` recipe and "Any option" accept raw Nix values; validation is only
  "parses as Nix", not type-checked against the option type.
- Package search uses the registry's `nixpkgs`, not the config's pinned
  input; versions shown may differ from what the flake will build.
- Home Manager options are not in the index (generic NixOS options only).

## Full user-account recipe (2026-10-15, Vincent's request)
The `user` recipe now covers everything NixOS lets you set on a user, in
sections (Basics / Access / Login & security / Advanced): name, full name,
account type (normal or system user with its own group), home + createHome,
uid, sudo (wheel), extra groups (chip suggestions + free text), shell
(bash/zsh/fish/nushell, program enabled automatically), per-user packages,
password (hashed server-side with `sha-crypt` SHA-512; `initial` →
initialHashedPassword, `fixed` → hashedPassword; empty → nothing written),
SSH authorized keys (one per line), linger, expires, users.mutableUsers.
Engine additions: FieldKinds `password`, `lines`, `packages`; `section` on
fields; `when` conditions support `!field`, `field=value` and `a&b`.
UI renders password inputs, textareas, chip lists, and section headings.

## Recipe catalog expanded (2026-10-15)
Added 36 recipes (71 total) in new categories Storage, Hardware, Nix,
Services plus more Network/Desktop/Containers/Tools: mount_drive, mount_smb
(credentials file / guest / inline), mount_nfs, swapfile, udisks, fstrim,
amd_gpu, intel_gpu, scanner, laptop_power, wake_on_lan, static_ip,
nameservers, xrdp, sunshine, kdeconnect, samba_server, hyprland, cinnamon,
xfce, fcitx5, keyd_capslock, console_keymap, browser, nix_ld, appimage,
binary_cache, nix_tuning, boot_limit, grub, kernel_params,
scheduled_command (systemd timer), boot_command, virtualbox, waydroid, adb.
Quoted attr-path segments (`fileSystems."/mnt/x"`, samba `"read only"`) work
through `split_attrpath`. The catalog test now applies every recipe to an
empty module and asserts the result parses, so a broken template fails CI.
Derived values: `writable_no` (samba), `shell_is_*`.

## Partition picker for "Mount a drive" (2026-10-15)
`crates/core/src/disks.rs` lists block devices without root: tree from
`lsblk -J`, UUID from /dev/disk/by-uuid symlinks, fstype/label/partlabel
from udev's db (/run/udev/data/b<maj>:<min>, major/minor from st_rdev);
partitions mounted at /, /boot, /nix or swap are flagged `system`. Route
`/api/machine/disks`. New FieldKind `partition` (value = UUID); the wizard
renders disks with their partitions, disables system/no-UUID ones, and
selecting one also fills the recipe's `fstype` select when it matches.

- 2026-10-16: recipe `nix_trusted_user` ("Trust a user for nix (trusted-users)",
  section Nix): one required Text field `user`, appends to
  `nix.settings.trusted-users`. Added so a target can be made permanently
  trusted after the first password-assisted deployment (see 14-remote.md).
  `nix_tuning` and `remote_builder_serve` also write that list.
