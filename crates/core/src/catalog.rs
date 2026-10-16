//! Curated wizard recipes: "turn on SSH", "add a user", "install Docker"...
//! Each recipe has a few fields and a set of templates that render into
//! [`ConfigChange`]s. Placeholders: `{{field}}` renders the field as a Nix
//! literal (strings quoted, bools/numbers bare, lists as `[ ... ]`);
//! `{{field:raw}}` inserts the text as typed (for attribute names, package
//! attrs, Nix expressions).

use crate::types::{ConfigChange, FieldKind, Recipe, RecipeField, RenderResult};
use crate::{CoreError, Result};
use std::collections::HashMap;

#[derive(Clone)]
enum Tpl {
    /// `key = value;` (both may contain placeholders).
    Set(&'static str, &'static str),
    /// Append one item to a list option.
    Append(&'static str, &'static str),
    /// Append each element of a List field to a list option; `{{item}}` is the element.
    AppendEach(&'static str, &'static str, &'static str),
    /// Add a package (attr may contain placeholders).
    Package(&'static str),
}

#[derive(Clone)]
struct Template {
    tpl: Tpl,
    /// Only emit when this Bool field is true / Text field non-empty.
    when: Option<&'static str>,
}

struct RecipeDef {
    meta: Recipe,
    templates: Vec<Template>,
}

fn f(name: &str, label: &str, kind: FieldKind, default: Option<&str>, help: Option<&str>) -> RecipeField {
    RecipeField {
        name: name.into(),
        label: label.into(),
        kind,
        default: default.map(|s| s.into()),
        help: help.map(|s| s.into()),
        placeholder: None,
        options: vec![],
        required: false,
        section: None,
    }
}
fn sec(mut field: RecipeField, section: &str) -> RecipeField {
    field.section = Some(section.into());
    field
}
fn req(mut field: RecipeField) -> RecipeField {
    field.required = true;
    field
}
fn sel(mut field: RecipeField, options: &[&str]) -> RecipeField {
    field.options = options.iter().map(|s| s.to_string()).collect();
    field
}
fn ph(mut field: RecipeField, p: &str) -> RecipeField {
    field.placeholder = Some(p.into());
    field
}
fn set(k: &'static str, v: &'static str) -> Template {
    Template { tpl: Tpl::Set(k, v), when: None }
}
fn set_if(w: &'static str, k: &'static str, v: &'static str) -> Template {
    Template { tpl: Tpl::Set(k, v), when: Some(w) }
}
fn append_if(w: &'static str, k: &'static str, v: &'static str) -> Template {
    Template { tpl: Tpl::Append(k, v), when: Some(w) }
}
fn append_each(field: &'static str, k: &'static str, v: &'static str) -> Template {
    Template { tpl: Tpl::AppendEach(field, k, v), when: Some(field) }
}
fn pkg_if(w: &'static str, a: &'static str) -> Template {
    Template { tpl: Tpl::Package(a), when: Some(w) }
}

fn recipe(id: &str, title: &str, description: &str, category: &str, keywords: &[&str], fields: Vec<RecipeField>, notes: Option<&str>, templates: Vec<Template>) -> RecipeDef {
    RecipeDef {
        meta: Recipe {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            category: category.into(),
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            fields,
            notes: notes.map(|s| s.into()),
        },
        templates,
    }
}

use FieldKind::*;

fn defs() -> Vec<RecipeDef> {
    vec![
        // ── Remote access & networking ─────────────────────────────────
        recipe("ssh", "Remote login with SSH", "Lets you log into this machine from another computer with the ssh command.", "Network",
            &["ssh", "openssh", "remote", "login", "sshd"],
            vec![
                f("password_auth", "Allow password login", Bool, Some("false"), Some("Off is safer: only SSH keys can log in.")),
                sel(f("root_login", "Allow root login", Select, Some("no"), None), &["no", "prohibit-password", "yes"]),
                f("port", "Port", Number, Some("22"), Some("Leave 22 unless you know why to change it.")),
            ],
            Some("The firewall port is opened automatically. Copy your public key with ssh-copy-id."),
            vec![
                set("services.openssh.enable", "true"),
                set("services.openssh.settings.PasswordAuthentication", "{{password_auth}}"),
                set("services.openssh.settings.PermitRootLogin", "{{root_login}}"),
                set("services.openssh.ports", "[ {{port}} ]"),
            ]),
        recipe("networkmanager", "Wi-Fi and network manager", "Turns on NetworkManager, the usual way to manage Wi-Fi and wired connections on a desktop.", "Network",
            &["wifi", "wireless", "network", "networkmanager", "ethernet"],
            vec![f("user", "Your username", Text, None, Some("Added to the networkmanager group so you can change connections without a password."))],
            None,
            vec![
                set("networking.networkmanager.enable", "true"),
                append_if("user", "users.users.{{user:raw}}.extraGroups", "\"networkmanager\""),
            ]),
        recipe("firewall", "Firewall", "Turns the firewall on and opens the ports you list.", "Network",
            &["firewall", "ports", "open port", "tcp", "udp"],
            vec![
                f("tcp", "TCP ports to open", List, None, Some("Comma separated, e.g. 80, 443")),
                f("udp", "UDP ports to open", List, None, None),
            ],
            None,
            vec![
                set("networking.firewall.enable", "true"),
                append_each("tcp", "networking.firewall.allowedTCPPorts", "{{item:raw}}"),
                append_each("udp", "networking.firewall.allowedUDPPorts", "{{item:raw}}"),
            ]),
        recipe("tailscale", "Tailscale VPN", "Private network between your devices through Tailscale.", "Network",
            &["tailscale", "vpn", "mesh"],
            vec![],
            Some("After rebuilding, run: sudo tailscale up"),
            vec![set("services.tailscale.enable", "true")]),
        recipe("avahi", "Discover devices on the local network", "mDNS/DNS-SD (Avahi): find printers, other computers and services by name, e.g. mypc.local.", "Network",
            &["avahi", "mdns", "bonjour", "zeroconf", ".local", "discovery"],
            vec![],
            None,
            vec![
                set("services.avahi.enable", "true"),
                set("services.avahi.nssmdns4", "true"),
                set("services.avahi.openFirewall", "true"),
            ]),
        recipe("syncthing", "Syncthing file sync", "Keeps folders in sync between your devices.", "Network",
            &["syncthing", "sync", "files", "backup"],
            vec![
                req(f("user", "Run as user", Text, None, Some("The user whose files are synced."))),
                f("open_ports", "Open the firewall for it", Bool, Some("true"), None),
            ],
            Some("Open http://127.0.0.1:8384 after rebuilding to pair devices."),
            vec![
                set("services.syncthing.enable", "true"),
                set("services.syncthing.user", "{{user}}"),
                set("services.syncthing.dataDir", "\"/home/{{user:raw}}\""),
                set_if("open_ports", "services.syncthing.openDefaultPorts", "true"),
            ]),

        // ── Containers & virtualization ────────────────────────────────
        recipe("docker", "Docker", "Run containers with Docker.", "Containers & VMs",
            &["docker", "container", "compose"],
            vec![
                f("user", "Your username", Text, None, Some("Added to the docker group so you can run docker without sudo.")),
                f("compose", "Also install docker-compose", Bool, Some("true"), None),
            ],
            Some("Log out and back in for the group change to take effect."),
            vec![
                set("virtualisation.docker.enable", "true"),
                append_if("user", "users.users.{{user:raw}}.extraGroups", "\"docker\""),
                pkg_if("compose", "docker-compose"),
            ]),
        recipe("podman", "Podman", "Run containers with Podman, a daemonless alternative to Docker.", "Containers & VMs",
            &["podman", "container"],
            vec![f("docker_compat", "Provide a 'docker' command alias", Bool, Some("true"), None)],
            None,
            vec![
                set("virtualisation.podman.enable", "true"),
                set("virtualisation.podman.dockerCompat", "{{docker_compat}}"),
                set("virtualisation.podman.defaultNetwork.settings.dns_enabled", "true"),
            ]),
        recipe("libvirt", "Virtual machines (virt-manager)", "KVM virtual machines with libvirt and the virt-manager app.", "Containers & VMs",
            &["vm", "virtual machine", "kvm", "qemu", "libvirt", "virt-manager"],
            vec![f("user", "Your username", Text, None, Some("Added to the libvirtd group."))],
            None,
            vec![
                set("virtualisation.libvirtd.enable", "true"),
                set("programs.virt-manager.enable", "true"),
                append_if("user", "users.users.{{user:raw}}.extraGroups", "\"libvirtd\""),
            ]),

        // ── Desktop ─────────────────────────────────────────────────────
        recipe("plasma", "KDE Plasma desktop", "The KDE Plasma 6 desktop with the SDDM login screen.", "Desktop",
            &["kde", "plasma", "desktop", "sddm"],
            vec![f("wayland", "Use Wayland (recommended)", Bool, Some("true"), None)],
            None,
            vec![
                set("services.displayManager.sddm.enable", "true"),
                set("services.displayManager.sddm.wayland.enable", "{{wayland}}"),
                set("services.desktopManager.plasma6.enable", "true"),
            ]),
        recipe("gnome", "GNOME desktop", "The GNOME desktop with the GDM login screen.", "Desktop",
            &["gnome", "desktop", "gdm"],
            vec![],
            None,
            vec![
                set("services.xserver.enable", "true"),
                set("services.displayManager.gdm.enable", "true"),
                set("services.desktopManager.gnome.enable", "true"),
            ]),
        recipe("autologin", "Log in automatically", "Skip the login screen and start the desktop as one user.", "Desktop",
            &["autologin", "auto login", "login"],
            vec![req(f("user", "Username", Text, None, None))],
            None,
            vec![
                set("services.displayManager.autoLogin.enable", "true"),
                set("services.displayManager.autoLogin.user", "{{user}}"),
            ]),
        recipe("sound", "Sound (PipeWire)", "Audio through PipeWire, with PulseAudio and ALSA compatibility.", "Desktop",
            &["sound", "audio", "pipewire", "pulseaudio", "speakers"],
            vec![],
            None,
            vec![
                set("security.rtkit.enable", "true"),
                set("services.pipewire.enable", "true"),
                set("services.pipewire.alsa.enable", "true"),
                set("services.pipewire.pulse.enable", "true"),
            ]),
        recipe("bluetooth", "Bluetooth", "Turns on Bluetooth, optionally with the Blueman tray app.", "Desktop",
            &["bluetooth", "headphones", "blueman"],
            vec![
                f("power_on_boot", "Turn on at boot", Bool, Some("true"), None),
                f("blueman", "Install the Blueman manager app", Bool, Some("false"), Some("Not needed on KDE or GNOME, which have their own.")),
            ],
            None,
            vec![
                set("hardware.bluetooth.enable", "true"),
                set("hardware.bluetooth.powerOnBoot", "{{power_on_boot}}"),
                set_if("blueman", "services.blueman.enable", "true"),
            ]),
        recipe("printing", "Printing", "CUPS printing, optionally with automatic discovery of network printers.", "Desktop",
            &["print", "printer", "cups"],
            vec![f("network", "Find network printers automatically", Bool, Some("true"), None)],
            None,
            vec![
                set("services.printing.enable", "true"),
                set_if("network", "services.avahi.enable", "true"),
                set_if("network", "services.avahi.nssmdns4", "true"),
            ]),
        recipe("flatpak", "Flatpak apps", "Install apps from Flathub alongside Nix packages.", "Desktop",
            &["flatpak", "flathub", "appstore"],
            vec![],
            Some("After rebuilding: flatpak remote-add --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo"),
            vec![set("services.flatpak.enable", "true")]),
        recipe("fonts", "Extra fonts", "Adds popular font packages.", "Desktop",
            &["fonts", "font", "nerd", "noto", "emoji"],
            vec![
                f("noto", "Noto (wide language coverage)", Bool, Some("true"), None),
                f("emoji", "Noto Color Emoji", Bool, Some("true"), None),
                f("nerd", "JetBrains Mono Nerd Font (for terminals)", Bool, Some("false"), None),
            ],
            None,
            vec![
                append_if("noto", "fonts.packages", "pkgs.noto-fonts"),
                append_if("emoji", "fonts.packages", "pkgs.noto-fonts-color-emoji"),
                append_if("nerd", "fonts.packages", "pkgs.nerd-fonts.jetbrains-mono"),
            ]),
        recipe("steam", "Steam and gaming", "Steam, with optional GameMode performance tweaks.", "Desktop",
            &["steam", "game", "gaming", "gamemode", "proton"],
            vec![
                f("remote_play", "Open firewall for Steam Remote Play", Bool, Some("false"), None),
                f("gamemode", "Enable GameMode", Bool, Some("true"), None),
            ],
            Some("Steam is unfree: the wizard also allows unfree packages."),
            vec![
                set("nixpkgs.config.allowUnfree", "true"),
                set("programs.steam.enable", "true"),
                set("programs.steam.remotePlay.openFirewall", "{{remote_play}}"),
                set_if("gamemode", "programs.gamemode.enable", "true"),
            ]),

        // ── System ──────────────────────────────────────────────────────
        recipe("hostname", "Computer name", "The name this machine shows on the network and in the terminal.", "System",
            &["hostname", "computer name", "name"],
            vec![req(ph(f("name", "Hostname", Text, None, Some("Letters, digits and dashes.")), "my-pc"))],
            None,
            vec![set("networking.hostName", "{{name}}")]),
        recipe("timezone", "Time zone", "Sets the system time zone.", "System",
            &["time", "timezone", "clock", "tz"],
            vec![req(ph(f("tz", "Time zone", Text, None, Some("Area/City from the tz database.")), "America/New_York"))],
            None,
            vec![set("time.timeZone", "{{tz}}")]),
        recipe("locale", "Language and region", "Default language and regional formats.", "System",
            &["locale", "language", "region", "i18n"],
            vec![req(ph(f("locale", "Locale", Text, Some("en_US.UTF-8"), None), "en_US.UTF-8"))],
            None,
            vec![set("i18n.defaultLocale", "{{locale}}")]),
        recipe("keyboard", "Keyboard layout", "Keyboard layout for the graphical session.", "System",
            &["keyboard", "layout", "xkb"],
            vec![
                req(f("layout", "Layout", Text, Some("us"), Some("e.g. us, de, fr"))),
                f("variant", "Variant", Text, None, Some("e.g. colemak, dvorak; leave empty for the default.")),
            ],
            None,
            vec![
                set("services.xserver.xkb.layout", "{{layout}}"),
                set_if("variant", "services.xserver.xkb.variant", "{{variant}}"),
            ]),
        recipe("user", "User account", "Creates or updates a user with everything NixOS lets you set: groups, shell, password, SSH keys, packages, home directory.", "System",
            &["user", "account", "login", "wheel", "sudo", "password", "ssh key", "groups", "shell"],
            vec![
                sec(req(ph(f("name", "Username", Text, None, Some("Lowercase letters, digits, - and _. No spaces.")), "alice")), "Basics"),
                sec(f("description", "Full name", Text, None, Some("Shown on the login screen.")), "Basics"),
                sec(sel(f("kind", "Account type", Select, Some("normal"), Some("A normal user logs in; a system user only runs services.")), &["normal", "system"]), "Basics"),
                sec(ph(f("home", "Home directory", Text, None, Some("Leave empty for /home/<username>.")), "/home/alice"), "Basics"),
                sec(f("uid", "User ID", Number, None, Some("Leave empty to pick automatically. Set it to keep file ownership stable across reinstalls.")), "Basics"),

                sec(f("sudo", "Can use sudo (administrator)", Bool, Some("true"), Some("Adds the wheel group.")), "Access"),
                sec(sel(f("groups", "Extra groups", List, Some("networkmanager"), Some("Tick the common ones or type more, comma separated.")),
                    &["networkmanager", "audio", "video", "docker", "libvirtd", "input", "dialout", "plugdev", "scanner", "lp", "kvm", "adbusers", "wireshark"]), "Access"),
                sec(sel(f("shell", "Shell", Select, Some("bash"), Some("zsh, fish and nushell are installed and enabled automatically.")), &["bash", "zsh", "fish", "nushell"]), "Access"),
                sec(ph(f("packages", "Packages just for this user", Packages, None, Some("Comma separated package names, e.g. firefox, vscode. Installed in the user's profile only.")), "firefox, vscode"), "Access"),

                sec(f("password", "Password", Password, None, Some("Stored as a SHA-512 hash; the plain text never reaches the file. Leave empty to set it later with: sudo passwd <username>")), "Login & security"),
                sec(sel(f("password_mode", "Password handling", Select, Some("initial"), Some("initial: used once at creation, the user may change it later. fixed: reapplied on every rebuild.")), &["initial", "fixed"]), "Login & security"),
                sec(ph(f("ssh_keys", "SSH public keys", Lines, None, Some("One key per line (the contents of id_ed25519.pub). Lets this user log in over SSH without a password.")), "ssh-ed25519 AAAA... alice@laptop"), "Login & security"),
                sec(f("linger", "Keep user services running when logged out", Bool, Some("false"), Some("For things like Syncthing or a Podman container started by this user.")), "Login & security"),
                sec(ph(f("expires", "Account expires on", Text, None, Some("YYYY-MM-DD, leave empty for never.")), "2027-12-31"), "Login & security"),

                sec(f("mutable", "Allow changing passwords with passwd", Bool, Some("true"), Some("Turning this off makes NixOS the only source of truth for users and passwords (users.mutableUsers = false).")), "Advanced"),
                sec(f("create_home", "Create the home directory if missing", Bool, Some("true"), None), "Advanced"),
            ],
            Some("If you left the password empty, set one after rebuilding: sudo passwd <username>"),
            vec![
                set_if("kind=normal", "users.users.{{name:raw}}.isNormalUser", "true"),
                set_if("kind=system", "users.users.{{name:raw}}.isSystemUser", "true"),
                set_if("kind=system", "users.users.{{name:raw}}.group", "{{name}}"),
                set_if("kind=system", "users.groups.{{name:raw}}", "{ }"),
                set_if("description", "users.users.{{name:raw}}.description", "{{description}}"),
                set_if("home", "users.users.{{name:raw}}.home", "{{home}}"),
                set_if("home", "users.users.{{name:raw}}.createHome", "{{create_home}}"),
                set_if("uid", "users.users.{{name:raw}}.uid", "{{uid}}"),
                append_if("sudo", "users.users.{{name:raw}}.extraGroups", "\"wheel\""),
                append_each("groups", "users.users.{{name:raw}}.extraGroups", "{{item}}"),
                set("users.users.{{name:raw}}.shell", "pkgs.{{shell:raw}}"),
                set_if("shell=zsh", "programs.zsh.enable", "true"),
                set_if("shell=fish", "programs.fish.enable", "true"),
                Template { tpl: Tpl::Package("nushell"), when: Some("shell=nushell") },
                set_if("packages", "users.users.{{name:raw}}.packages", "{{packages}}"),
                Template { tpl: Tpl::Set("users.users.{{name:raw}}.initialHashedPassword", "{{password}}"), when: Some("password&password_mode=initial") },
                Template { tpl: Tpl::Set("users.users.{{name:raw}}.hashedPassword", "{{password}}"), when: Some("password&password_mode=fixed") },
                set_if("ssh_keys", "users.users.{{name:raw}}.openssh.authorizedKeys.keys", "{{ssh_keys}}"),
                set_if("linger", "users.users.{{name:raw}}.linger", "true"),
                set_if("expires", "users.users.{{name:raw}}.expires", "{{expires}}"),
                set_if("!mutable", "users.mutableUsers", "false"),
            ]),
        recipe("allow_unfree", "Allow unfree software", "Lets you install proprietary packages such as Steam, Discord, or NVIDIA drivers.", "System",
            &["unfree", "proprietary", "license", "nvidia", "discord"],
            vec![],
            None,
            vec![set("nixpkgs.config.allowUnfree", "true")]),
        recipe("flakes", "Enable flakes", "Turns on the nix command and flakes features.", "System",
            &["flakes", "nix-command", "experimental"],
            vec![],
            None,
            vec![
                Template { tpl: Tpl::Append("nix.settings.experimental-features", "\"nix-command\""), when: None },
                Template { tpl: Tpl::Append("nix.settings.experimental-features", "\"flakes\""), when: None },
            ]),
        recipe("gc", "Clean up old generations automatically", "Deletes old system generations on a schedule to free disk space.", "System",
            &["gc", "garbage", "clean", "disk space", "generations"],
            vec![
                sel(f("when", "How often", Select, Some("weekly"), None), &["daily", "weekly", "monthly"]),
                f("days", "Keep generations newer than (days)", Number, Some("14"), None),
            ],
            None,
            vec![
                set("nix.gc.automatic", "true"),
                set("nix.gc.dates", "{{when}}"),
                set("nix.gc.options", "\"--delete-older-than {{days:raw}}d\""),
            ]),
        recipe("auto_upgrade", "Automatic updates", "Rebuilds the system from the channel on a schedule.", "System",
            &["update", "upgrade", "automatic"],
            vec![f("reboot", "Reboot automatically when the kernel changes", Bool, Some("false"), None)],
            None,
            vec![
                set("system.autoUpgrade.enable", "true"),
                set("system.autoUpgrade.allowReboot", "{{reboot}}"),
            ]),
        recipe("zram", "Compressed swap in RAM (zram)", "Adds compressed swap in memory; helps machines with little RAM.", "System",
            &["swap", "zram", "memory", "ram"],
            vec![f("percent", "Percent of RAM to use", Number, Some("50"), None)],
            None,
            vec![
                set("zramSwap.enable", "true"),
                set("zramSwap.memoryPercent", "{{percent}}"),
            ]),
        recipe("latest_kernel", "Latest Linux kernel", "Uses the newest kernel instead of the LTS default (better for very new hardware).", "System",
            &["kernel", "linux", "latest"],
            vec![],
            None,
            vec![set("boot.kernelPackages", "pkgs.linuxPackages_latest")]),
        recipe("systemd_boot", "Boot loader: systemd-boot (UEFI)", "The simple boot loader for UEFI machines.", "System",
            &["boot", "bootloader", "uefi", "systemd-boot"],
            vec![],
            None,
            vec![
                set("boot.loader.systemd-boot.enable", "true"),
                set("boot.loader.efi.canTouchEfiVariables", "true"),
            ]),
        recipe("fwupd", "Firmware updates", "Update device firmware (UEFI, SSDs, peripherals) with fwupd.", "System",
            &["firmware", "fwupd", "bios"],
            vec![],
            Some("Check for updates with: fwupdmgr get-updates"),
            vec![set("services.fwupd.enable", "true")]),

        // ── Tools ───────────────────────────────────────────────────────
        recipe("zsh", "Zsh shell", "Installs Zsh and makes it the default shell for all users.", "Tools",
            &["zsh", "shell", "terminal"],
            vec![f("default", "Make it the default shell", Bool, Some("true"), None)],
            None,
            vec![
                set("programs.zsh.enable", "true"),
                set_if("default", "users.defaultUserShell", "pkgs.zsh"),
            ]),
        recipe("git", "Git", "Installs Git and sets your name and email for commits.", "Tools",
            &["git", "version control"],
            vec![
                f("name", "Your name", Text, None, None),
                f("email", "Your email", Text, None, None),
            ],
            None,
            vec![
                set("programs.git.enable", "true"),
                set_if("name", "programs.git.config.user.name", "{{name}}"),
                set_if("email", "programs.git.config.user.email", "{{email}}"),
            ]),
        recipe("nvidia", "NVIDIA graphics driver", "Proprietary NVIDIA driver with modesetting; requires unfree packages.", "Hardware",
            &["nvidia", "gpu", "graphics", "driver"],
            vec![f("open", "Use the open kernel module (Turing or newer GPUs)", Bool, Some("false"), None)],
            Some("Reboot after switching."),
            vec![
                set("nixpkgs.config.allowUnfree", "true"),
                set("hardware.graphics.enable", "true"),
                set("services.xserver.videoDrivers", "[ \"nvidia\" ]"),
                set("hardware.nvidia.modesetting.enable", "true"),
                set("hardware.nvidia.open", "{{open}}"),
            ]),
        // ── Storage ─────────────────────────────────────────────────────
        recipe("mount_drive", "Mount a drive", "Mounts an internal or external disk partition at boot.", "Storage",
            &["mount", "disk", "drive", "partition", "fstab", "uuid"],
            vec![
                req(ph(f("mountpoint", "Mount at", Text, None, Some("A folder path; it is created if missing.")), "/mnt/data")),
                req(ph(f("uuid", "Partition", Partition, None, Some("Pick the partition; its UUID is used so the mount survives device renames.")), "c532a972-420f-43d0-8726-d272a28330a3")),
                sel(f("fstype", "File system", Select, Some("ext4"), None), &["ext4", "btrfs", "xfs", "ntfs", "vfat", "exfat"]),
                f("nofail", "Do not block boot if the drive is missing", Bool, Some("true"), Some("Recommended for external drives.")),
                f("users", "Let any user mount and unmount it", Bool, Some("false"), None),
            ],
            None,
            vec![
                set("fileSystems.\"{{mountpoint:raw}}\".device", "\"/dev/disk/by-uuid/{{uuid:raw}}\""),
                set("fileSystems.\"{{mountpoint:raw}}\".fsType", "{{fstype}}"),
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"defaults\""), when: None },
                append_if("nofail", "fileSystems.\"{{mountpoint:raw}}\".options", "\"nofail\""),
                append_if("users", "fileSystems.\"{{mountpoint:raw}}\".options", "\"users\""),
                Template { tpl: Tpl::Package("ntfs3g"), when: Some("fstype=ntfs") },
                Template { tpl: Tpl::Package("exfatprogs"), when: Some("fstype=exfat") },
            ]),
        recipe("mount_smb", "Mount a network share (SMB / Windows / NAS)", "Mounts a shared folder from a NAS or Windows machine, connecting on first access.", "Storage",
            &["smb", "cifs", "samba", "nas", "share", "windows", "synology", "unraid"],
            vec![
                req(ph(f("server", "Server address", Text, None, None), "10.0.0.5 or nas.local")),
                req(ph(f("share", "Share name", Text, None, None), "media")),
                req(ph(f("mountpoint", "Mount at", Text, None, None), "/mnt/nas/media")),
                sel(f("auth", "Login", Select, Some("managed"), Some("managed: a login stored encrypted in this app's secrets.age (recommended). credentials_file: a file you create by hand. inline: plain text in your configuration (not recommended).")), &["managed", "credentials_file", "guest", "inline"]),
                f("secret", "Managed secret", Secret, None, Some("An SMB login from the Secrets page. It is installed as a root-only file at build time.")),
                ph(f("credentials", "Credentials file", Text, Some("/etc/nixos/smb-secrets"), Some("Two lines: username=... and password=... ; create it by hand, outside git.")), "/etc/nixos/smb-secrets"),
                f("username", "Username (inline login only)", Text, None, None),
                f("password", "Password (inline login only)", Text, None, Some("Stored in plain text in your configuration. Prefer a credentials file.")),
                f("uid", "Owner user ID", Number, Some("1000"), Some("Files appear owned by this user; 1000 is usually the first user.")),
                f("gid", "Owner group ID", Number, Some("100"), Some("100 is the users group.")),
            ],
            Some("The share is mounted the first time the folder is accessed, so a missing NAS never blocks boot."),
            vec![
                Template { tpl: Tpl::Package("cifs-utils"), when: None },
                set("fileSystems.\"{{mountpoint:raw}}\".device", "\"//{{server:raw}}/{{share:raw}}\""),
                set("fileSystems.\"{{mountpoint:raw}}\".fsType", "\"cifs\""),
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"x-systemd.automount\""), when: None },
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"noauto\""), when: None },
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"x-systemd.idle-timeout=60\""), when: None },
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"uid={{uid:raw}}\""), when: None },
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"gid={{gid:raw}}\""), when: None },
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"credentials={{credentials:raw}}\""), when: Some("auth=credentials_file") },
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"credentials=/var/lib/nixos-manager/secrets/{{secret:raw}}\""), when: Some("auth=managed&secret") },
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"guest\""), when: Some("auth=guest") },
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"username={{username:raw}}\""), when: Some("auth=inline") },
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"password={{password:raw}}\""), when: Some("auth=inline") },
            ]),
        recipe("mount_nfs", "Mount an NFS share", "Mounts a folder exported over NFS from another Linux machine or NAS.", "Storage",
            &["nfs", "share", "nas", "export"],
            vec![
                req(ph(f("server", "Server address", Text, None, None), "10.0.0.5")),
                req(ph(f("export", "Exported path", Text, None, None), "/volume1/media")),
                req(ph(f("mountpoint", "Mount at", Text, None, None), "/mnt/media")),
            ],
            None,
            vec![
                set("fileSystems.\"{{mountpoint:raw}}\".device", "\"{{server:raw}}:{{export:raw}}\""),
                set("fileSystems.\"{{mountpoint:raw}}\".fsType", "\"nfs\""),
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"x-systemd.automount\""), when: None },
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"noauto\""), when: None },
                Template { tpl: Tpl::Append("fileSystems.\"{{mountpoint:raw}}\".options", "\"nofail\""), when: None },
            ]),
        recipe("swapfile", "Swap file", "Adds a swap file of a given size (for hibernation or machines with little RAM).", "Storage",
            &["swap", "swapfile", "memory"],
            vec![
                f("size", "Size in MiB", Number, Some("8192"), Some("8192 = 8 GiB.")),
                ph(f("path", "File", Text, Some("/swapfile"), None), "/swapfile"),
            ],
            None,
            vec![Template { tpl: Tpl::Append("swapDevices", "{ device = {{path}}; size = {{size}}; }"), when: None }]),
        recipe("udisks", "Automatic mounting of USB drives", "Lets the desktop mount USB sticks and external drives when plugged in.", "Storage",
            &["usb", "automount", "udisks", "gvfs", "external"],
            vec![],
            None,
            vec![set("services.udisks2.enable", "true"), set("services.gvfs.enable", "true")]),
        recipe("fstrim", "SSD maintenance (TRIM)", "Runs fstrim weekly so SSDs keep their performance.", "Storage",
            &["ssd", "trim", "fstrim", "discard"],
            vec![],
            None,
            vec![set("services.fstrim.enable", "true")]),

        // ── Hardware ────────────────────────────────────────────────────
        recipe("amd_gpu", "AMD graphics", "Open-source AMD driver with early loading and Vulkan.", "Hardware",
            &["amd", "radeon", "amdgpu", "gpu", "graphics", "vulkan"],
            vec![f("bits32", "Also 32-bit graphics (needed for Steam and Wine)", Bool, Some("true"), None)],
            None,
            vec![
                set("hardware.graphics.enable", "true"),
                set_if("bits32", "hardware.graphics.enable32Bit", "true"),
                set("hardware.amdgpu.initrd.enable", "true"),
            ]),
        recipe("intel_gpu", "Intel graphics", "Intel driver with hardware video decoding.", "Hardware",
            &["intel", "gpu", "graphics", "vaapi", "video decoding"],
            vec![f("bits32", "Also 32-bit graphics (needed for Steam and Wine)", Bool, Some("true"), None)],
            None,
            vec![
                set("hardware.graphics.enable", "true"),
                set_if("bits32", "hardware.graphics.enable32Bit", "true"),
                Template { tpl: Tpl::Append("hardware.graphics.extraPackages", "pkgs.intel-media-driver"), when: None },
                set("environment.sessionVariables.LIBVA_DRIVER_NAME", "\"iHD\""),
            ]),
        recipe("scanner", "Scanner support", "Enables SANE so scanners and multifunction printers work.", "Hardware",
            &["scanner", "sane", "scan", "multifunction"],
            vec![f("user", "Your username", Text, None, Some("Added to the scanner and lp groups."))],
            None,
            vec![
                set("hardware.sane.enable", "true"),
                append_if("user", "users.users.{{user:raw}}.extraGroups", "\"scanner\""),
                append_if("user", "users.users.{{user:raw}}.extraGroups", "\"lp\""),
            ]),
        recipe("laptop_power", "Laptop power saving", "Battery-friendly power management and CPU thermal control.", "Hardware",
            &["laptop", "battery", "power", "tlp", "thermald", "lid"],
            vec![
                sel(f("manager", "Power manager", Select, Some("power-profiles-daemon"), Some("power-profiles-daemon integrates with KDE and GNOME; TLP squeezes more battery on other desktops.")), &["power-profiles-daemon", "tlp"]),
                f("thermald", "Thermal control (Intel CPUs)", Bool, Some("true"), None),
                sel(f("lid", "When the lid closes", Select, Some("suspend"), None), &["suspend", "ignore", "lock", "poweroff", "hibernate"]),
            ],
            None,
            vec![
                set_if("manager=power-profiles-daemon", "services.power-profiles-daemon.enable", "true"),
                set_if("manager=tlp", "services.tlp.enable", "true"),
                set_if("manager=tlp", "services.power-profiles-daemon.enable", "false"),
                set_if("thermald", "services.thermald.enable", "true"),
                set("services.logind.lidSwitch", "{{lid}}"),
            ]),
        recipe("wake_on_lan", "Wake-on-LAN", "Lets this machine be woken up over the network.", "Hardware",
            &["wol", "wake on lan", "wake", "network"],
            vec![req(ph(f("interface", "Network interface", Text, None, Some("Find it with: ip link")), "enp14s0"))],
            None,
            vec![set("networking.interfaces.{{interface:raw}}.wakeOnLan.enable", "true")]),

        // ── Network (more) ──────────────────────────────────────────────
        recipe("static_ip", "Static IP address", "Gives this machine a fixed address instead of DHCP.", "Network",
            &["static ip", "ip address", "gateway", "dhcp"],
            vec![
                req(ph(f("interface", "Network interface", Text, None, Some("Find it with: ip link")), "enp14s0")),
                req(ph(f("address", "IP address", Text, None, None), "192.168.1.50")),
                f("prefix", "Prefix length", Number, Some("24"), Some("24 = 255.255.255.0")),
                req(ph(f("gateway", "Gateway", Text, None, None), "192.168.1.1")),
                f("dns", "DNS servers", List, Some("1.1.1.1, 9.9.9.9"), None),
            ],
            None,
            vec![
                set("networking.useDHCP", "false"),
                Template { tpl: Tpl::Append("networking.interfaces.{{interface:raw}}.ipv4.addresses", "{ address = {{address}}; prefixLength = {{prefix}}; }"), when: None },
                set("networking.defaultGateway", "{{gateway}}"),
                append_each("dns", "networking.nameservers", "{{item}}"),
            ]),
        recipe("nameservers", "DNS servers", "Use specific DNS servers instead of the ones from your router.", "Network",
            &["dns", "nameserver", "resolver"],
            vec![req(f("dns", "DNS servers", List, Some("1.1.1.1, 9.9.9.9"), None))],
            None,
            vec![append_each("dns", "networking.nameservers", "{{item}}")]),
        recipe("xrdp", "Remote desktop (RDP)", "Lets you open a desktop session on this machine from Windows Remote Desktop or any RDP client.", "Network",
            &["rdp", "xrdp", "remote desktop", "windows"],
            vec![sel(f("desktop", "Desktop to start", Select, Some("plasma"), None), &["plasma", "gnome", "xfce", "cinnamon"])],
            Some("Connect to port 3389 with the user's normal login."),
            vec![
                set("services.xrdp.enable", "true"),
                set("services.xrdp.openFirewall", "true"),
                set_if("desktop=plasma", "services.xrdp.defaultWindowManager", "\"startplasma-x11\""),
                set_if("desktop=gnome", "services.xrdp.defaultWindowManager", "\"gnome-session\""),
                set_if("desktop=xfce", "services.xrdp.defaultWindowManager", "\"xfce4-session\""),
                set_if("desktop=cinnamon", "services.xrdp.defaultWindowManager", "\"cinnamon-session\""),
            ]),
        recipe("sunshine", "Sunshine game streaming", "Streams this machine's screen to Moonlight clients (TVs, phones, handhelds).", "Network",
            &["sunshine", "moonlight", "stream", "game streaming"],
            vec![],
            Some("Pair clients at https://localhost:47990 after rebuilding."),
            vec![
                set("services.sunshine.enable", "true"),
                set("services.sunshine.autoStart", "true"),
                set("services.sunshine.capSysAdmin", "true"),
                set("services.sunshine.openFirewall", "true"),
            ]),
        recipe("kdeconnect", "KDE Connect / phone integration", "Pairs with your phone for notifications, file sharing and remote input.", "Network",
            &["kdeconnect", "phone", "android", "gsconnect"],
            vec![],
            None,
            vec![set("programs.kdeconnect.enable", "true")]),
        recipe("samba_server", "Share a folder on the network (Samba)", "Shares a folder so Windows, macOS and other machines can open it.", "Network",
            &["samba", "share", "smb", "server", "file sharing"],
            vec![
                req(ph(f("name", "Share name", Text, None, None), "public")),
                req(ph(f("path", "Folder to share", Text, None, None), "/srv/share")),
                f("writable", "Writable", Bool, Some("true"), None),
                f("guest", "Allow access without a password", Bool, Some("false"), None),
            ],
            Some("Users need a Samba password: sudo smbpasswd -a <username>"),
            vec![
                set("services.samba.enable", "true"),
                set("services.samba.openFirewall", "true"),
                set("services.samba.settings.\"{{name:raw}}\".path", "{{path}}"),
                set("services.samba.settings.\"{{name:raw}}\".browseable", "\"yes\""),
                set("services.samba.settings.\"{{name:raw}}\".\"read only\"", "{{writable_no}}"),
                set_if("guest", "services.samba.settings.\"{{name:raw}}\".\"guest ok\"", "\"yes\""),
                set("services.samba-wsdd.enable", "true"),
                set("services.samba-wsdd.openFirewall", "true"),
            ]),

        // ── Desktop (more) ──────────────────────────────────────────────
        recipe("hyprland", "Hyprland window manager", "The Hyprland Wayland compositor, with the usual portal for screen sharing.", "Desktop",
            &["hyprland", "wayland", "tiling", "window manager"],
            vec![],
            Some("Hyprland is configured in ~/.config/hypr/hyprland.conf, not here."),
            vec![
                set("programs.hyprland.enable", "true"),
                set("programs.hyprland.xwayland.enable", "true"),
            ]),
        recipe("cinnamon", "Cinnamon desktop", "The Cinnamon desktop (as on Linux Mint) with the LightDM login screen.", "Desktop",
            &["cinnamon", "mint", "desktop"],
            vec![],
            None,
            vec![
                set("services.xserver.enable", "true"),
                set("services.xserver.displayManager.lightdm.enable", "true"),
                set("services.xserver.desktopManager.cinnamon.enable", "true"),
            ]),
        recipe("xfce", "XFCE desktop", "The lightweight XFCE desktop with the LightDM login screen.", "Desktop",
            &["xfce", "lightweight", "desktop"],
            vec![],
            None,
            vec![
                set("services.xserver.enable", "true"),
                set("services.xserver.displayManager.lightdm.enable", "true"),
                set("services.xserver.desktopManager.xfce.enable", "true"),
            ]),
        recipe("fcitx5", "Input method (Vietnamese, Chinese, Japanese, Korean...)", "Type in other scripts with fcitx5 and the engines you pick.", "Desktop",
            &["input method", "fcitx", "vietnamese", "unikey", "bamboo", "chinese", "japanese", "korean", "ime"],
            vec![
                sel(f("engines", "Engines", List, Some("unikey"), Some("unikey and bamboo: Vietnamese · mozc: Japanese · hangul: Korean · chinese-addons / rime: Chinese")), &["unikey", "bamboo", "mozc", "hangul", "chinese-addons", "rime"]),
            ],
            Some("Pick the engine in the fcitx5 tray icon after logging in."),
            vec![
                set("i18n.inputMethod.enable", "true"),
                set("i18n.inputMethod.type", "\"fcitx5\""),
                set("i18n.inputMethod.fcitx5.waylandFrontend", "true"),
                append_each("engines", "i18n.inputMethod.fcitx5.addons", "pkgs.fcitx5-{{item:raw}}"),
            ]),
        recipe("keyd_capslock", "Remap Caps Lock", "Turns Caps Lock into Escape, Control, or both (Escape when tapped, Control when held).", "Desktop",
            &["capslock", "caps lock", "remap", "keyd", "escape", "control"],
            vec![sel(f("to", "Caps Lock becomes", Select, Some("overload(control, esc)"), None), &["overload(control, esc)", "esc", "leftcontrol"])],
            None,
            vec![
                set("services.keyd.enable", "true"),
                set("services.keyd.keyboards.default.ids", "[ \"*\" ]"),
                set("services.keyd.keyboards.default.settings.main.capslock", "{{to}}"),
            ]),
        recipe("console_keymap", "Console keyboard layout", "Keyboard layout for the text console (before the desktop starts).", "Desktop",
            &["console", "keymap", "tty", "keyboard"],
            vec![req(ph(f("keymap", "Keymap", Text, Some("us"), Some("e.g. us, de, fr, uk, dvorak")), "us"))],
            None,
            vec![set("console.keyMap", "{{keymap}}")]),
        recipe("browser", "Web browser", "Installs a browser as a proper program (with system integration).", "Desktop",
            &["browser", "firefox", "chromium", "chrome", "web"],
            vec![sel(f("which", "Browser", Select, Some("firefox"), None), &["firefox", "chromium"])],
            None,
            vec![
                set_if("which=firefox", "programs.firefox.enable", "true"),
                set_if("which=chromium", "programs.chromium.enable", "true"),
                Template { tpl: Tpl::Package("chromium"), when: Some("which=chromium") },
            ]),

        // ── Nix & boot ──────────────────────────────────────────────────
        recipe("nix_ld", "Run downloaded Linux programs (nix-ld)", "Lets binaries that were not built for NixOS run (VS Code remote server, proprietary tools, pip wheels).", "Nix",
            &["nix-ld", "binary", "dynamic", "not found", "vscode", "pip", "ld-linux"],
            vec![],
            Some("Programs that still fail usually need a library added to programs.nix-ld.libraries."),
            vec![set("programs.nix-ld.enable", "true")]),
        recipe("appimage", "AppImage support", "Run .AppImage files by double-clicking or from the terminal.", "Nix",
            &["appimage", "portable", "run"],
            vec![],
            None,
            vec![set("programs.appimage.enable", "true"), set("programs.appimage.binfmt", "true")]),
        recipe("binary_cache", "Extra binary cache", "Download prebuilt packages from an additional cache instead of compiling (e.g. nix-community, cachix).", "Nix",
            &["cache", "cachix", "substituter", "binary cache", "compile", "slow build"],
            vec![
                req(ph(f("url", "Cache URL", Text, Some("https://nix-community.cachix.org"), None), "https://nix-community.cachix.org")),
                req(ph(f("key", "Public key", Text, Some("nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="), Some("Shown on the cache's page.")), "name.cachix.org-1:...")),
            ],
            None,
            vec![
                Template { tpl: Tpl::Append("nix.settings.substituters", "{{url}}"), when: None },
                Template { tpl: Tpl::Append("nix.settings.trusted-public-keys", "{{key}}"), when: None },
            ]),
        recipe("nix_tuning", "Nix store and build settings", "Deduplicate the store automatically and control how many builds run in parallel.", "Nix",
            &["nix", "optimise", "dedupe", "max-jobs", "cores", "store", "disk"],
            vec![
                f("optimise", "Deduplicate the store automatically", Bool, Some("true"), Some("Saves disk space; builds become slightly slower.")),
                f("max_jobs", "Parallel builds", Text, Some("auto"), Some("auto, or a number.")),
                f("trusted", "Trusted users (can use extra caches)", List, Some("@wheel"), None),
            ],
            None,
            vec![
                set_if("optimise", "nix.settings.auto-optimise-store", "true"),
                set("nix.settings.max-jobs", "{{max_jobs}}"),
                append_each("trusted", "nix.settings.trusted-users", "{{item}}"),
            ]),
        recipe("nix_trusted_user", "Trust a user for nix (trusted-users)", "Lets a user hand unsigned builds to this machine's nix daemon and use extra caches. Needed on a machine that receives \"build here, deploy to target\" deployments as that user; without it the app must grant trust temporarily on every deployment.", "Nix",
            &["trusted", "trusted-users", "trusted user", "deploy", "remote", "lacks a signature", "nix daemon", "sudo"],
            vec![
                req(ph(f("user", "User to trust", Text, None, Some("The user deployments log in as over SSH. @wheel trusts everyone in the wheel group. A trusted user is effectively root for the nix store.")), "vincent")),
            ],
            Some("Deploy this change once (the build dialog asks for the target's sudo password to grant trust temporarily); from then on the user is trusted permanently and no password is needed for that step."),
            vec![Template { tpl: Tpl::Append("nix.settings.trusted-users", "{{user}}"), when: None }]),
        recipe("boot_limit", "Limit boot menu entries", "Keeps only the newest generations in the boot menu so it stays short and the boot partition does not fill up.", "Nix",
            &["boot", "generations", "menu", "limit", "boot partition full"],
            vec![f("keep", "Entries to keep", Number, Some("10"), None)],
            None,
            vec![set("boot.loader.systemd-boot.configurationLimit", "{{keep}}")]),
        recipe("grub", "Boot loader: GRUB (BIOS or dual boot)", "GRUB for legacy BIOS machines, or when you want a boot menu that also finds Windows.", "Nix",
            &["grub", "bios", "bootloader", "dual boot", "windows"],
            vec![
                req(ph(f("device", "Install to disk", Text, Some("/dev/sda"), Some("The whole disk, not a partition. Use \"nodev\" for UEFI.")), "/dev/sda")),
                f("efi", "UEFI machine", Bool, Some("false"), None),
                f("os_prober", "Detect other operating systems (Windows)", Bool, Some("true"), None),
            ],
            None,
            vec![
                set("boot.loader.grub.enable", "true"),
                set("boot.loader.grub.device", "{{device}}"),
                set_if("efi", "boot.loader.grub.efiSupport", "true"),
                set_if("efi", "boot.loader.efi.canTouchEfiVariables", "true"),
                set_if("os_prober", "boot.loader.grub.useOSProber", "true"),
                set("boot.loader.systemd-boot.enable", "false"),
            ]),
        recipe("kernel_params", "Kernel parameters", "Extra parameters passed to the Linux kernel at boot.", "Nix",
            &["kernel", "parameters", "cmdline", "boot options", "quiet"],
            vec![req(ph(f("params", "Parameters", List, None, Some("Comma separated, e.g. quiet, amd_pstate=active")), "quiet, splash"))],
            None,
            vec![append_each("params", "boot.kernelParams", "{{item}}")]),

        // ── Services ────────────────────────────────────────────────────
        recipe("scheduled_command", "Run a command on a schedule", "A systemd timer that runs a command daily, weekly, or on any calendar expression.", "Services",
            &["cron", "schedule", "timer", "systemd", "backup", "periodic"],
            vec![
                req(ph(f("name", "Job name", Text, None, Some("Letters, digits and dashes.")), "backup-home")),
                req(ph(f("command", "Command", Text, None, Some("Full path or a package's binary, e.g. ${pkgs.rsync}/bin/rsync -a /home /mnt/backup")), "${pkgs.rsync}/bin/rsync -a /home /mnt/backup")),
                sel(f("when", "Run", Select, Some("daily"), Some("A systemd calendar expression also works, e.g. Mon *-*-* 03:00")), &["hourly", "daily", "weekly", "monthly"]),
                f("user", "Run as user", Text, Some("root"), None),
                f("persistent", "Catch up if the machine was off", Bool, Some("true"), None),
            ],
            Some("Check it with: systemctl list-timers"),
            vec![
                set("systemd.services.{{name:raw}}.description", "\"{{name:raw}} (scheduled by Nixos-manager)\""),
                set("systemd.services.{{name:raw}}.script", "\"{{command:raw}}\""),
                set("systemd.services.{{name:raw}}.serviceConfig.Type", "\"oneshot\""),
                set("systemd.services.{{name:raw}}.serviceConfig.User", "{{user}}"),
                set("systemd.timers.{{name:raw}}.wantedBy", "[ \"timers.target\" ]"),
                set("systemd.timers.{{name:raw}}.timerConfig.OnCalendar", "{{when}}"),
                set("systemd.timers.{{name:raw}}.timerConfig.Persistent", "{{persistent}}"),
            ]),
        recipe("boot_command", "Run a command at boot", "A systemd service that runs a command once at every boot.", "Services",
            &["startup", "boot", "service", "autostart", "systemd"],
            vec![
                req(ph(f("name", "Service name", Text, None, None), "set-fan-curve")),
                req(ph(f("command", "Command", Text, None, Some("Full path or ${pkgs.something}/bin/...")), "${pkgs.coreutils}/bin/echo hello")),
                f("user", "Run as user", Text, Some("root"), None),
            ],
            None,
            vec![
                set("systemd.services.{{name:raw}}.description", "\"{{name:raw}} (added by Nixos-manager)\""),
                set("systemd.services.{{name:raw}}.wantedBy", "[ \"multi-user.target\" ]"),
                set("systemd.services.{{name:raw}}.after", "[ \"network.target\" ]"),
                set("systemd.services.{{name:raw}}.script", "\"{{command:raw}}\""),
                set("systemd.services.{{name:raw}}.serviceConfig.Type", "\"oneshot\""),
                set("systemd.services.{{name:raw}}.serviceConfig.User", "{{user}}"),
                set("systemd.services.{{name:raw}}.serviceConfig.RemainAfterExit", "true"),
            ]),
        recipe("virtualbox", "VirtualBox", "Run virtual machines with VirtualBox.", "Containers & VMs",
            &["virtualbox", "vbox", "vm"],
            vec![f("user", "Your username", Text, None, Some("Added to the vboxusers group (USB passthrough)."))],
            None,
            vec![
                set("virtualisation.virtualbox.host.enable", "true"),
                append_if("user", "users.users.{{user:raw}}.extraGroups", "\"vboxusers\""),
            ]),
        recipe("waydroid", "Android apps (Waydroid)", "Runs Android apps in a container (Wayland desktops only).", "Containers & VMs",
            &["android", "waydroid", "apk"],
            vec![],
            Some("After rebuilding: sudo waydroid init"),
            vec![set("virtualisation.waydroid.enable", "true")]),
        recipe("adb", "Android developer tools (adb)", "adb and fastboot with permissions to talk to a plugged-in phone.", "Tools",
            &["adb", "android", "fastboot", "phone"],
            vec![f("user", "Your username", Text, None, Some("Added to the adbusers group."))],
            None,
            vec![
                set("programs.adb.enable", "true"),
                append_if("user", "users.users.{{user:raw}}.extraGroups", "\"adbusers\""),
            ]),
        // ── Builds ──────────────────────────────────────────────────────
        recipe("remote_builder_use", "Use another machine as a builder", "Offload compiling to a faster machine over SSH (nix.buildMachines). Evaluation and activation stay local.", "Builds",
            &["remote builder", "distributed", "build machine", "offload", "buildMachines"],
            vec![
                req(ph(f("host", "Builder address", Text, None, Some("Hostname or IP reachable over SSH.")), "desktop1-nixos")),
                f("user", "SSH user on the builder", Text, Some("remotebuild"), Some("Created on the builder by the companion recipe.")),
                sel(f("system", "Builder architecture", Select, Some("x86_64-linux"), None), &["x86_64-linux", "aarch64-linux"]),
                f("max_jobs", "Parallel jobs to send", Number, Some("4"), None),
                f("speed", "Speed factor", Number, Some("2"), Some("Relative to this machine; higher means preferred.")),
                sel(f("features", "Features the builder offers", List, Some("big-parallel, kvm, nixos-test"), None), &["big-parallel", "kvm", "nixos-test", "benchmark"]),
                f("key", "Root's SSH private key on this machine", Text, Some("/root/.ssh/id_ed25519"), Some("The nix daemon connects as root. Generate it on the Remote page.")),
                f("only_remote", "Never build locally (max-jobs = 0)", Bool, Some("false"), Some("Everything is sent to the builder; local builds only when it is unreachable? No: they fail. Keep off for laptops.")),
            ],
            Some("On the builder, run the recipe \"Let this machine build for others\" with root's public key from this machine's Remote page."),
            vec![
                set("nix.distributedBuilds", "true"),
                set("nix.settings.builders-use-substitutes", "true"),
                Template { tpl: Tpl::Append("nix.buildMachines", "{\n      hostName = {{host}};\n      sshUser = {{user}};\n      sshKey = {{key}};\n      system = {{system}};\n      protocol = \"ssh-ng\";\n      maxJobs = {{max_jobs}};\n      speedFactor = {{speed}};\n      supportedFeatures = {{features}};\n    }"), when: None },
                set_if("only_remote", "nix.settings.max-jobs", "0"),
            ]),
        recipe("remote_builder_serve", "Let this machine build for others", "Creates a builder account other machines can use over SSH to compile here.", "Builds",
            &["remote builder", "distributed", "serve builds", "trusted-users"],
            vec![
                f("user", "Builder user", Text, Some("remotebuild"), None),
                req(ph(f("keys", "Public keys of the machines allowed to build here", Lines, None, Some("root's public key from each client machine (Remote page → builder key), one per line.")), "ssh-ed25519 AAAA... root@laptop nix-builder")),
            ],
            Some("Make sure SSH is enabled here (recipe \"Remote login with SSH\")."),
            vec![
                set("users.users.{{user:raw}}.isNormalUser", "true"),
                set("users.users.{{user:raw}}.createHome", "false"),
                set("users.users.{{user:raw}}.group", "{{user}}"),
                set("users.groups.{{user:raw}}", "{ }"),
                set("users.users.{{user:raw}}.openssh.authorizedKeys.keys", "{{keys}}"),
                Template { tpl: Tpl::Append("nix.settings.trusted-users", "{{user}}"), when: None },
                set("services.openssh.enable", "true"),
            ]),
        recipe("custom", "Custom setting", "Set any option by hand: an attribute path and a Nix value.", "Advanced",
            &["custom", "option", "raw", "advanced"],
            vec![
                req(ph(f("key", "Option", Text, None, Some("Attribute path, e.g. services.openssh.enable")), "services.foo.enable")),
                req(ph(f("value", "Value (Nix)", Text, None, Some("Written as Nix: true, 42, \"text\", [ 1 2 ], pkgs.foo")), "true")),
            ],
            None,
            vec![set("{{key:raw}}", "{{value:raw}}")]),
    ]
}

pub fn recipes() -> Vec<Recipe> {
    defs().into_iter().map(|d| d.meta).collect()
}

pub fn render(recipe_id: &str, values: &HashMap<String, String>, package_set: Option<&str>) -> Result<RenderResult> {
    let def = defs().into_iter().find(|d| d.meta.id == recipe_id).ok_or_else(|| CoreError::NotFound(format!("recipe {recipe_id}")))?;
    let mut vals: HashMap<String, (FieldKind, String)> = HashMap::new();
    let mut warnings = Vec::new();
    for field in &def.meta.fields {
        let raw = values.get(&field.name).cloned().or_else(|| field.default.clone()).unwrap_or_default();
        let raw = raw.trim().to_string();
        if field.required && raw.is_empty() {
            return Err(CoreError::Invalid(format!("'{}' is required", field.label)));
        }
        if field.kind == FieldKind::Number && !raw.is_empty() && raw.parse::<i64>().is_err() {
            return Err(CoreError::Invalid(format!("'{}' must be a whole number", field.label)));
        }
        if field.kind == FieldKind::Packages && !raw.is_empty() {
            for item in raw.split(',').map(|x| x.trim()).filter(|x| !x.is_empty()) {
                if crate::edit::validate_expr(item).is_err() {
                    return Err(CoreError::Invalid(format!("'{item}' is not a valid package name")));
                }
            }
        }
        if field.kind == FieldKind::Select && !raw.is_empty() && !field.options.contains(&raw) {
            return Err(CoreError::Invalid(format!("'{}' must be one of {}", field.label, field.options.join(", "))));
        }
        if field.kind == FieldKind::Text && field.name == "name" && !raw.is_empty() && !raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            warnings.push(format!("'{raw}' contains characters that may not be valid for a name"));
        }
        vals.insert(field.name.clone(), (field.kind, raw));
    }
    if recipe_id == "mount_smb" && vals.get("auth").map(|(_, v)| v == "managed").unwrap_or(false) && !truthy_val(&vals, "secret") {
        return Err(CoreError::Invalid("pick a managed secret, or choose another login method".into()));
    }
    // Derived flags used by some recipes.
    if let Some((_, w)) = vals.get("writable").cloned() {
        vals.insert("writable_no".into(), (FieldKind::Text, if w == "true" { "no".into() } else { "yes".into() }));
    }
    if let Some((_, shell)) = vals.get("shell").cloned() {
        vals.insert("shell_is_zsh".into(), (FieldKind::Bool, (shell == "zsh").to_string()));
        vals.insert("shell_is_fish".into(), (FieldKind::Bool, (shell == "fish").to_string()));
    }

    let truthy = |name: &str| -> bool {
        match vals.get(name) {
            Some((FieldKind::Bool, v)) => v == "true",
            Some((_, v)) => !v.trim().is_empty(),
            None => false,
        }
    };
    // `when` conditions: "field" (truthy), "!field", "field=value".
    let holds = |cond: &str| -> bool {
        if cond.contains('&') {
            return cond.split('&').all(|c| {
                let c = c.trim();
                if let Some(rest) = c.strip_prefix('!') {
                    return !truthy(rest);
                }
                if let Some((f, v)) = c.split_once('=') {
                    return vals.get(f).map(|(_, x)| x.trim() == v).unwrap_or(false);
                }
                truthy(c)
            });
        }
        if let Some(rest) = cond.strip_prefix('!') {
            return !truthy(rest);
        }
        if let Some((f, v)) = cond.split_once('=') {
            return vals.get(f).map(|(_, x)| x.trim() == v).unwrap_or(false);
        }
        truthy(cond)
    };
    let subst = |text: &str, extra: Option<(&str, &str)>| -> Result<String> {
        let mut out = text.to_string();
        let mut guard = 0;
        while let Some(start) = out.find("{{") {
            guard += 1;
            if guard > 50 {
                break;
            }
            let end = out[start..].find("}}").map(|e| start + e).ok_or_else(|| CoreError::Other("bad template".into()))?;
            let inner = &out[start + 2..end];
            let (name, raw) = match inner.split_once(':') {
                Some((n, "raw")) => (n, true),
                _ => (inner, false),
            };
            let rendered = if let Some((_, ev)) = extra.filter(|(en, _)| *en == name) {
                if raw { ev.to_string() } else { nix_literal(FieldKind::Text, ev) }
            } else {
                let (kind, v) = vals.get(name).ok_or_else(|| CoreError::Other(format!("unknown field {name}")))?;
                if raw { v.clone() } else { nix_literal(*kind, v) }
            };
            out.replace_range(start..end + 2, &rendered);
        }
        Ok(out)
    };

    let set = package_set.map(|s| s.trim()).filter(|s| !s.is_empty() && *s != "pkgs");
    let reset = |text: String| -> String {
        match set {
            Some(ps) => text.replace("pkgs.", &format!("{ps}.")),
            None => text,
        }
    };
    let mut changes: Vec<ConfigChange> = Vec::new();
    for t in &def.templates {
        if let Some(w) = t.when {
            if !holds(w) {
                continue;
            }
        }
        match &t.tpl {
            Tpl::Set(k, v) => changes.push(ConfigChange::Set { key: subst(k, None)?, value: reset(subst(v, None)?) }),
            Tpl::Append(k, v) => changes.push(ConfigChange::AppendList { key: subst(k, None)?, item: reset(subst(v, None)?) }),
            Tpl::AppendEach(field, k, v) => {
                let (_, list) = vals.get(*field).cloned().unwrap_or((FieldKind::List, String::new()));
                for item in list.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    changes.push(ConfigChange::AppendList { key: subst(k, None)?, item: subst(v, Some(("item", item)))? });
                }
            }
            Tpl::Package(a) => changes.push(ConfigChange::AddPackage { attr: subst(a, None)?, set: set.map(|s| s.to_string()) }),
        }
    }
    let preview = preview_of(&changes);
    Ok(RenderResult { changes, preview, warnings, notes: def.meta.notes.clone() })
}

fn truthy_val(vals: &HashMap<String, (FieldKind, String)>, name: &str) -> bool {
    vals.get(name).map(|(_, v)| !v.trim().is_empty()).unwrap_or(false)
}

pub fn nix_literal(kind: FieldKind, v: &str) -> String {
    match kind {
        FieldKind::Bool => if v == "true" { "true".into() } else { "false".into() },
        FieldKind::Number => v.trim().to_string(),
        FieldKind::Text | FieldKind::Select | FieldKind::Partition | FieldKind::Secret => crate::edit::nix_string(v),
        FieldKind::Password => crate::edit::nix_string(&hash_password(v)),
        FieldKind::List => {
            let items: Vec<String> = v.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).map(crate::edit::nix_string).collect();
            format!("[ {} ]", items.join(" "))
        }
        FieldKind::Lines => {
            let items: Vec<String> = v.lines().map(|s| s.trim()).filter(|s| !s.is_empty()).map(crate::edit::nix_string).collect();
            format!("[ {} ]", items.join(" "))
        }
        FieldKind::Packages => {
            let items: Vec<String> = v
                .split(',')
                .map(|s| s.trim().trim_start_matches("pkgs."))
                .filter(|s| !s.is_empty())
                .map(|s| format!("pkgs.{s}"))
                .collect();
            format!("[ {} ]", items.join(" "))
        }
    }
}

/// SHA-512 crypt hash (`$6$...`) as accepted by users.users.<n>.hashedPassword.
/// Already-hashed input (starting with `$`) is passed through.
pub fn hash_password(plain: &str) -> String {
    if plain.starts_with('$') {
        return plain.to_string();
    }
    let params = sha_crypt::Sha512Params::new(5000).expect("valid rounds");
    sha_crypt::sha512_simple(plain, &params).expect("hash")
}

/// A readable snippet of the changes, for the preview step. Appends to the
/// same list are merged into one line.
pub fn preview_of(changes: &[ConfigChange]) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut appends: Vec<(String, Vec<String>)> = Vec::new();
    let mut packages = Vec::new();
    for c in changes {
        match c {
            ConfigChange::Set { key, value } => lines.push(format!("{key} = {value};")),
            ConfigChange::AppendList { key, item } => match appends.iter_mut().find(|(k, _)| k == key) {
                Some((_, items)) => items.push(item.clone()),
                None => {
                    lines.push(format!("\u{0}{key}")); // placeholder keeps the original position
                    appends.push((key.clone(), vec![item.clone()]));
                }
            },
            ConfigChange::AddPackage { attr, set } => packages.push(match set.as_deref() {
                Some(s) if s != "pkgs" => format!("{s}.{attr}"),
                _ => attr.clone(),
            }),
        }
    }
    let mut out: Vec<String> = lines
        .into_iter()
        .map(|l| match l.strip_prefix('\u{0}') {
            Some(key) => {
                let items = appends.iter().find(|(k, _)| k == key).map(|(_, v)| v.join(" ")).unwrap_or_default();
                format!("{key} = [ {items} ];  # added to the list")
            }
            None => l,
        })
        .collect();
    if !packages.is_empty() {
        out.push(format!("environment.systemPackages = with pkgs; [ {} ];  # added to the list", packages.join(" ")));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_recipes_render_with_defaults() {
        for r in recipes() {
            let mut values = HashMap::new();
            for f in &r.fields {
                if f.required && f.default.is_none() {
                    values.insert(f.name.clone(), match f.kind { FieldKind::Number => "1".to_string(), FieldKind::List => "a, b".to_string(), _ => f.placeholder.clone().unwrap_or_else(|| "alice".into()) });
                }
            }
            if r.id == "mount_smb" {
                values.insert("secret".into(), "nas".into());
            }
            let res = render(&r.id, &values, None).unwrap_or_else(|e| panic!("{}: {e}", r.id));
            assert!(!res.changes.is_empty(), "{} produced nothing", r.id);
            if r.id == "mount_smb" {
                assert!(res.preview.contains("credentials=/var/lib/nixos-manager/secrets/nas"), "{}", res.preview);
            }
            // Every change must apply to an empty module and yield valid Nix.
            let out = crate::edit::apply_changes("{ config, pkgs, ... }:\n{\n}\n", &res.changes).unwrap_or_else(|e| panic!("{}: apply failed: {e}\n{}", r.id, res.preview));
            assert!(rnix::Root::parse(&out).errors().is_empty(), "{} produced invalid Nix:\n{out}", r.id);
        }
    }

    #[test]
    fn user_recipe() {
        let mut v = HashMap::new();
        v.insert("name".into(), "bob".into());
        v.insert("shell".into(), "zsh".into());
        v.insert("groups".into(), "audio, video".into());
        v.insert("password".into(), "hunter2".into());
        v.insert("ssh_keys".into(), "ssh-ed25519 AAAA bob@pc\n\nssh-rsa BBBB".into());
        v.insert("packages".into(), "firefox, pkgs.vscode".into());
        v.insert("mutable".into(), "false".into());
        let res = render("user", &v, None).unwrap();
        assert!(res.changes.contains(&ConfigChange::Set { key: "users.users.bob.isNormalUser".into(), value: "true".into() }));
        assert!(res.changes.contains(&ConfigChange::AppendList { key: "users.users.bob.extraGroups".into(), item: "\"wheel\"".into() }));
        assert!(res.changes.contains(&ConfigChange::AppendList { key: "users.users.bob.extraGroups".into(), item: "\"video\"".into() }));
        assert!(res.changes.contains(&ConfigChange::Set { key: "users.users.bob.shell".into(), value: "pkgs.zsh".into() }));
        assert!(res.changes.contains(&ConfigChange::Set { key: "programs.zsh.enable".into(), value: "true".into() }));
        assert!(res.changes.contains(&ConfigChange::Set { key: "users.users.bob.openssh.authorizedKeys.keys".into(), value: "[ \"ssh-ed25519 AAAA bob@pc\" \"ssh-rsa BBBB\" ]".into() }));
        assert!(res.changes.contains(&ConfigChange::Set { key: "users.users.bob.packages".into(), value: "[ pkgs.firefox pkgs.vscode ]".into() }));
        assert!(res.changes.contains(&ConfigChange::Set { key: "users.mutableUsers".into(), value: "false".into() }));
        let pw = res.changes.iter().find_map(|c| if let ConfigChange::Set { key, value } = c { if key == "users.users.bob.initialHashedPassword" { Some(value.clone()) } else { None } } else { None }).expect("hashed password");
        assert!(pw.starts_with("\"$6$"), "{pw}");
        assert!(!res.preview.contains("hunter2"));
        assert!(!res.changes.iter().any(|c| matches!(c, ConfigChange::Set { key, .. } if key.ends_with(".hashedPassword") && !key.contains("initial"))));
        assert!(res.preview.contains("users.users.bob.isNormalUser = true;"));
        // no password → no password settings at all
        let mut v2 = HashMap::new();
        v2.insert("name".into(), "cy".into());
        let res2 = render("user", &v2, None).unwrap();
        assert!(!res2.changes.iter().any(|c| matches!(c, ConfigChange::Set { key, .. } if key.contains("Password"))));
        assert!(hash_password("$6$abc$def") == "$6$abc$def");
    }

    #[test]
    fn validation() {
        let mut v = HashMap::new();
        v.insert("port".into(), "abc".into());
        assert!(render("ssh", &v, None).is_err());
        assert!(render("hostname", &HashMap::new(), None).is_err());
        assert!(render("nope", &HashMap::new(), None).is_err());
        let mut v = HashMap::new();
        v.insert("name".into(), "bob".into());
        v.insert("shell".into(), "zsh".into());
        let res = render("user", &v, Some("pkgs-unstable")).unwrap();
        assert!(res.changes.contains(&ConfigChange::Set { key: "users.users.bob.shell".into(), value: "pkgs-unstable.zsh".into() }));
        let res = render("docker", &HashMap::new(), Some("pkgs-25_11")).unwrap();
        assert!(res.changes.contains(&ConfigChange::AddPackage { attr: "docker-compose".into(), set: Some("pkgs-25_11".into()) }));
        assert!(res.preview.contains("pkgs-25_11.docker-compose"));
    }
}
