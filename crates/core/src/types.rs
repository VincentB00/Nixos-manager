//! API types shared between the Rust side and the Angular UI.
//! Every type here derives `TS` so `cargo test -p nixos-manager-core` can
//! regenerate `ui/src/app/api/generated/*.ts`. Never hand-write mirrors.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ts_rs::TS;

// ───────────────────────── app state ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct AppState {
    pub version: u32,
    pub last_location: Option<String>,
    pub recent_locations: Vec<RecentLocation>,
    pub prefs: Prefs,
    /// Remote machines this app can deploy to with nixos-rebuild --target-host.
    #[serde(default)]
    pub deploy_targets: Vec<DeployTarget>,
    /// Working folders whose hardware-configuration.nix was copied from a
    /// deploy target (keyed by folder path). The sync-on-open leaves such a
    /// file alone while its content still matches the recorded hash.
    #[serde(default)]
    pub hardware_pulls: std::collections::HashMap<String, HardwarePull>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HardwarePull {
    pub target: String,
    pub sha256: String,
    /// RFC 3339 timestamp.
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HardwarePullRequest {
    pub target: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HardwarePullResult {
    pub target: String,
    /// "etc" (the target's /etc/nixos file) or "generate" (nixos-generate-config on the target).
    pub source: String,
    pub rel_path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeployTarget {
    /// Display name, e.g. "cyber-server-1".
    pub name: String,
    /// SSH destination, e.g. "root@10.0.0.5" or "vincent@server".
    pub ssh_target: String,
    /// nixosConfigurations.<name> to build for it (defaults to the selected host).
    pub flake_host: Option<String>,
    /// Build on this machine instead of locally (nixos-rebuild --build-host).
    pub build_host: Option<String>,
    /// Pass --sudo for activation (needed when ssh_target is not root).
    pub use_remote_sudo: bool,
    /// SSH port when not 22 (passed as `-p` to every ssh/rsync call).
    #[serde(default)]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecentLocation {
    pub path: String,
    /// RFC 3339 timestamp of the last time this location was opened.
    pub last_opened: String,
    pub host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct Prefs {
    pub theme: Option<String>,
    pub default_action: Option<String>,
}

// ───────────────────────── filesystem browsing ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DirEntryInfo {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub is_nix: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BrowseResult {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<DirEntryInfo>,
    /// True when this directory looks like a NixOS configuration root.
    pub looks_like_config: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LocationSuggestion {
    pub path: String,
    pub kind: ConfigKind,
    pub reason: String,
}

// ───────────────────────── location / config ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum ConfigKind {
    /// Has a flake.nix at the root.
    Flake,
    /// No flake, but a configuration.nix.
    Legacy,
    /// Neither; probably not a NixOS config.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LocationInfo {
    pub path: String,
    pub kind: ConfigKind,
    /// flake.nix or configuration.nix, relative to `path`.
    pub entry_file: Option<String>,
    pub has_hardware_config: bool,
    pub has_flake_lock: bool,
    pub is_etc_nixos: bool,
    pub is_git: bool,
    pub writable: bool,
    /// Host names found statically in flake.nix (nixosConfigurations.<name>).
    /// For a legacy config this is empty.
    pub hosts: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "kebab-case")]
pub enum FileKind {
    Flake,
    FlakeLock,
    Configuration,
    HardwareConfiguration,
    Module,
    Script,
    Secret,
    Data,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum ImportVia {
    /// Listed in an `imports = [ ... ]` list.
    Imports,
    /// Referenced as a path somewhere else (readFile, module arg, ...).
    Reference,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImportRef {
    /// Normalised path relative to the config root.
    pub target: String,
    /// The path literal as written in the source.
    pub raw: String,
    pub via: ImportVia,
    /// False when the import is commented out.
    pub enabled: bool,
    pub exists: bool,
    pub line: u32,
    pub node_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileSummary {
    pub rel_path: String,
    pub kind: FileKind,
    pub size: u64,
    pub parse_ok: bool,
    pub parse_errors: Vec<String>,
    pub imports: Vec<ImportRef>,
    pub imported_by: Vec<String>,
    /// Top-level option namespaces this file touches, e.g. "services.docker".
    pub namespaces: Vec<String>,
    /// First comment in the file, used as a human description.
    pub description: Option<String>,
    /// Module function arguments, e.g. ["config", "pkgs", "pkgs-unstable"].
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImportEdge {
    pub from: String,
    pub to: String,
    pub via: ImportVia,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConfigOverview {
    pub location: LocationInfo,
    pub files: Vec<FileSummary>,
    /// Every directory under the root (relative), including empty ones.
    pub directories: Vec<String>,
    pub edges: Vec<ImportEdge>,
    /// .nix files nobody imports: entry points.
    pub roots: Vec<String>,
    /// .nix files that exist but are not reachable from any root through
    /// enabled imports.
    pub unreachable: Vec<String>,
    pub machine: MachineInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct MachineInfo {
    pub hostname: Option<String>,
    pub nixos_version: Option<String>,
    pub nix_version: Option<String>,
    /// The daemon / TUI itself runs as root (run.sh started with sudo): root
    /// actions need no sudo password.
    #[serde(default)]
    pub running_as_root: bool,
}

// ───────────────────────── nix file tree ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Span {
    /// Byte offsets into the file.
    pub start: u32,
    pub end: u32,
    /// 1-based lines.
    pub line: u32,
    pub end_line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// `{ ... }`
    Attrset,
    /// `[ ... ]`
    List,
    Bool,
    Int,
    Float,
    /// String literal. `interpolated` on the node tells whether it has `${}`.
    String,
    /// `./foo.nix`, `/abs`, `~/x`, `<nixpkgs>`
    Path,
    /// An identifier or attribute selection used as a value, e.g. `pkgs.git`.
    Ref,
    Null,
    /// `inherit a b;` / `inherit (x) a b;`
    Inherit,
    /// `let ... in body`; children are the bindings then one child labelled "in".
    Letin,
    /// `a ++ b`; two children.
    Concat,
    /// `a // b`; two children.
    Merge,
    /// A function `x: body` or `{ a, b }: body`.
    Function,
    /// Anything the form view cannot represent. `source` has the text.
    Code,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Node {
    /// Stable within one file version: "<start>-<end>".
    pub id: String,
    pub kind: NodeKind,
    /// Attribute key for bindings (e.g. "services.docker.enable"),
    /// "in" for a let body, None for list items.
    pub label: Option<String>,
    /// Whole entry including key and `;` for bindings, the item for list items.
    pub span: Span,
    /// The value expression only (what SetScalar/SetRaw replace).
    pub value_span: Span,
    /// Display value for scalars (unquoted string, number, path text, ref text).
    pub value: Option<String>,
    /// Raw source for Code nodes and for anything the UI may want to show.
    pub source: Option<String>,
    pub children: Vec<Node>,
    /// Leading and/or trailing comment text, trimmed, joined with newlines.
    pub comment: Option<String>,
    /// `with pkgs;` wrapping this value.
    pub with_namespace: Option<String>,
    /// String has `${...}` interpolation.
    pub interpolated: bool,
    /// Item is commented out (only for list items and imports).
    pub disabled: bool,
    /// Whether SetScalar is allowed on this node.
    pub editable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ParseErrorInfo {
    pub message: String,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NixFileView {
    pub rel_path: String,
    /// Content hash; edits must send it back and are rejected if it changed.
    pub version: String,
    pub parse_errors: Vec<ParseErrorInfo>,
    /// Module function arguments if the file is `{ a, b, ... }: body`.
    pub args: Vec<String>,
    pub has_ellipsis: bool,
    pub root: Node,
    pub source: String,
    pub line_count: u32,
}

// ───────────────────────── edits ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ScalarValue {
    Bool { value: bool },
    Int {
        #[ts(type = "number")]
        value: i64,
    },
    Float { value: f64 },
    String { value: String },
    Path { value: String },
    Ref { value: String },
    Null,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum EditOp {
    /// Replace a scalar value with a typed literal.
    SetScalar { node_id: String, value: ScalarValue },
    /// Replace a node's value expression with arbitrary Nix source.
    SetRaw { node_id: String, source: String },
    /// Append an item to a list.
    ListAppend { node_id: String, source: String },
    /// Remove a list item (its whole line when it is alone on it).
    ListRemove { node_id: String },
    /// Comment out / uncomment a list item.
    ToggleDisabled { node_id: String },
    /// Add `key = source;` inside an attrset (or the module body).
    AddBinding { node_id: String, key: String, source: String },
    /// Remove a binding and its line(s).
    RemoveBinding { node_id: String },
    /// Replace the whole file (code fallback).
    ReplaceFile { source: String },
    /// Make sure the module function receives `name` (e.g. "vars").
    EnsureArg { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EditRequest {
    pub path: String,
    pub file: String,
    pub version: String,
    pub op: EditOp,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FsRequest {
    pub path: String,
    /// Target relative to the config root.
    pub target: String,
    /// New name/location for rename and duplicate.
    pub to: Option<String>,
    /// Delete folders with contents.
    pub recursive: Option<bool>,
    /// Rename: rewrite path literals in files that import/reference the target (default true).
    pub update_references: Option<bool>,
    /// Delete: also remove `imports` entries pointing at the target (default false).
    #[serde(default)]
    pub remove_references: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RenameResult {
    pub from: String,
    pub to: String,
    /// .nix files whose path literals were rewritten.
    pub updated_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NewFileRequest {
    pub path: String,
    pub file: String,
    pub source: String,
    /// When set, also add `./file` to this file's imports list.
    pub import_into: Option<String>,
}

// ───────────────────────── jobs ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum LogStream {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LogLine {
    pub stream: LogStream,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobInfo {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub status: JobStatus,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub error: Option<String>,
    pub log_lines: u32,
    /// Where the log is also written on disk (survives a reboot / daemon restart).
    #[serde(default)]
    pub log_file: Option<String>,
}

/// A job log kept on disk (see JobManager::history).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobLogFile {
    pub name: String,
    pub title: String,
    pub status: Option<String>,
    pub started_at: Option<String>,
    pub size: u64,
}

/// What to do with the machine after a successful build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum AfterBuild {
    None,
    Poweroff,
    Reboot,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct JobDetail {
    pub info: JobInfo,
    pub log: Vec<LogLine>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum JobEvent {
    Log { job_id: String, line: LogLine },
    Status { job_id: String, status: JobStatus, error: Option<String> },
    Created { job: JobInfo },
}

// ───────────────────────── deploy ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "kebab-case")]
pub enum RebuildAction {
    Switch,
    Boot,
    Test,
    Build,
    DryBuild,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum KeepConfiguration {
    System,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum HardwareChoice {
    /// Keep /etc/nixos/hardware-configuration.nix (copied over the working copy's).
    System,
    /// Apply the working copy's changed file (explicitly confirmed).
    Workspace,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HardwareStatus {
    pub system_exists: bool,
    pub local_exists: bool,
    /// Where the working copy keeps the file (usually "hardware-configuration.nix").
    pub rel_path: String,
    pub differs: bool,
    pub unified_diff: Option<String>,
    /// Set when the working copy's file is (still) the one copied from a deploy target.
    pub pulled_from: Option<HardwarePull>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
#[ts(export)]
pub enum HardwareSyncStatus {
    Copied,
    Refreshed,
    Unchanged,
    /// Left alone: it is the file copied from a deploy target (see HardwarePull).
    FromTarget,
    NoSystemFile,
    NotWritable,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HardwareSync {
    pub status: HardwareSyncStatus,
    pub rel_path: String,
    pub message: String,
    /// A flake.lock found in the working folder was removed (it lives in /etc/nixos only).
    #[serde(default)]
    pub lock_removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConfigConflict {
    pub system_source: String,
    pub local_source: String,
    pub unified_diff: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeployPreflight {
    /// See `MachineInfo::running_as_root`: no local sudo password is needed.
    #[serde(default)]
    pub running_as_root: bool,
    pub etc_nixos_exists: bool,
    pub etc_nixos_readable: bool,
    /// The working copy *is* /etc/nixos; sync must be refused (sync.sh guard).
    pub source_is_etc_nixos: bool,
    pub etc_has_flake_lock: bool,
    pub etc_has_hardware_config: bool,
    pub local_has_hardware_config: bool,
    /// Working copy vs /etc/nixos hardware-configuration.nix (this machine).
    pub hardware: HardwareStatus,
    pub configuration_conflict: Option<ConfigConflict>,
    pub hosts: Vec<String>,
    pub current_hostname: Option<String>,
    pub kind: ConfigKind,
    pub secrets: SecretsDeployInfo,
    /// An SSH agent is visible to the daemon (keys with passphrases will work).
    pub ssh_agent: bool,
    /// The app's own passphrase-less key (~/.ssh/nixos-manager_ed25519) exists.
    pub app_key_exists: bool,
    /// Hosts that have a managed block in ~/.ssh/config (set up with "Set up key access").
    pub app_key_hosts: Vec<String>,
    pub targets: Vec<DeployTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeployRequest {
    pub path: String,
    pub host: Option<String>,
    pub action: RebuildAction,
    /// sync.sh --upgrade: do not preserve flake.lock.
    pub upgrade: bool,
    /// Required when preflight reported a configuration.nix conflict.
    pub keep_configuration: Option<KeepConfiguration>,
    /// Which hardware-configuration.nix a build on this machine applies when
    /// the working copy's differs from /etc/nixos. None = system (the
    /// source of truth; the working copy's file is replaced).
    pub hardware: Option<HardwareChoice>,
    /// Skip the sync step and only run nixos-rebuild on /etc/nixos.
    pub skip_sync: bool,
    /// Sudo password. Held in memory for this job only, never persisted.
    pub password: String,
    /// Password of secrets.age; when given, managed secrets are installed
    /// into /var/lib/nixos-manager/secrets before nixos-rebuild.
    pub secrets_password: Option<String>,
    /// When set: build here, activate on that machine (no sync, no local sudo).
    pub target: Option<DeployTarget>,
    /// When set (and `target` is not): compile on that machine with
    /// `nixos-rebuild --build-host`, then activate here (normal sync + local sudo).
    #[serde(default)]
    pub build_host: Option<DeployTarget>,
    /// Power off or reboot after a SUCCESSFUL build (this machine, or the
    /// target for remote deployments). Nothing happens on failure.
    #[serde(default)]
    pub after: Option<AfterBuild>,
    /// Do the `after` action even when the build fails (never after a manual stop).
    #[serde(default)]
    pub after_even_on_failure: bool,
    /// With `target`: what to do with THIS machine after the deployment
    /// (`after` applies to the target then). Uses logind (`systemctl
    /// poweroff|reboot`), falling back to sudo with `password` when given.
    #[serde(default)]
    pub after_local: Option<AfterBuild>,
    /// With `target`: copy the working copy to the target over SSH and run
    /// the normal sync + nixos-rebuild THERE (instead of building here and
    /// sending the result). Needs sudo on the target, no nix trust.
    #[serde(default)]
    pub on_target: bool,
    /// On-target mode: export NIX_CONFIG (flakes) and NIXPKGS_ALLOW_UNFREE=1
    /// (with --impure for flakes) so a target whose configuration does not
    /// enable them yet can still build.
    #[serde(default)]
    pub allow_unfree_env: bool,
    /// sudo password of the SSH user on the target. Used for the temporary
    /// nix trust (only if the user is not trusted) and for activation; not
    /// needed when the user is root or has passwordless sudo there.
    pub target_sudo_password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Generation {
    pub number: u32,
    pub date: String,
    pub current: bool,
    pub nixos_version: Option<String>,
    pub kernel: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApiError {
    pub error: String,
    pub code: String,
}

// ───────────────────────── option docs ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OptionDoc {
    pub name: String,
    pub description: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub default: Option<String>,
    pub example: Option<String>,
    pub declarations: Vec<String>,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, Default)]
#[ts(export)]
pub struct OptionsStatus {
    pub ready: bool,
    pub building: bool,
    pub count: u32,
    pub error: Option<String>,
    /// Where the index came from (store path).
    pub source: Option<String>,
    pub job_id: Option<String>,
}

// ───────────────────────── packages ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PackageInfo {
    /// Attribute under pkgs, e.g. "firefox" or "python3Packages.requests".
    pub attr: String,
    pub pname: String,
    pub version: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PackageSearchResult {
    pub query: String,
    /// Package set the results belong to.
    pub set: String,
    pub results: Vec<PackageInfo>,
    pub truncated: bool,
    /// True when this was the first search and nix had to build its cache.
    pub slow: bool,
}

// ───────────────────────── wizard / recipes ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    Bool,
    Text,
    Number,
    /// Comma-separated values in the UI; `options` are suggestions.
    List,
    Select,
    /// Hashed server-side (SHA-512 crypt); the file only gets the hash.
    Password,
    /// One entry per line (e.g. SSH public keys).
    Lines,
    /// Comma-separated package attrs, rendered as `[ pkgs.a pkgs.b ]`.
    Packages,
    /// A partition UUID chosen from the machine's disks (picker in the UI).
    Partition,
    /// Name of a managed secret (picker in the UI).
    Secret,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RecipeField {
    pub name: String,
    pub label: String,
    pub kind: FieldKind,
    pub default: Option<String>,
    pub help: Option<String>,
    pub placeholder: Option<String>,
    /// Choices for Select fields; suggestions for List fields.
    pub options: Vec<String>,
    pub required: bool,
    /// Group heading in the form, e.g. "Access".
    pub section: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Recipe {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub keywords: Vec<String>,
    pub fields: Vec<RecipeField>,
    /// Shown after applying (e.g. "run passwd to set the password").
    pub notes: Option<String>,
}

/// One change the wizard wants to make to a module file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConfigChange {
    /// `key = value;` — replaces an existing value at that path, or adds it.
    Set { key: String, value: String },
    /// Append `item` to the list at `key` (created if missing).
    AppendList { key: String, item: String },
    /// Add a package to environment.systemPackages, from `set` (default "pkgs").
    AddPackage { attr: String, set: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RenderRequest {
    pub recipe_id: String,
    pub values: HashMap<String, String>,
    /// Package set to take packages from (e.g. "pkgs-unstable"); default "pkgs".
    pub package_set: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RenderResult {
    pub changes: Vec<ConfigChange>,
    /// Human-readable Nix snippet of what will be added.
    pub preview: String,
    pub warnings: Vec<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApplyChangesRequest {
    pub path: String,
    pub file: String,
    pub version: String,
    pub changes: Vec<ConfigChange>,
}

// ───────────────────────── flake ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FlakeFollow {
    /// The nested input being overridden, e.g. "nixpkgs".
    pub input: String,
    /// What it follows, e.g. "nixpkgs".
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LockedInput {
    pub rev: Option<String>,
    /// RFC 3339 date of the locked commit.
    pub date: Option<String>,
    /// Human-readable source, e.g. "github:NixOS/nixpkgs/nixos-unstable".
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FlakeInput {
    pub name: String,
    pub url: Option<String>,
    pub follows: Vec<FlakeFollow>,
    pub locked: Option<LockedInput>,
    /// Extra-channel inputs are named nixpkgs-<channel>.
    pub is_nixpkgs_channel: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FlakeVar {
    pub name: String,
    /// Nix source of the value.
    pub value: String,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FlakeInfo {
    pub version: String,
    pub description: Option<String>,
    pub inputs: Vec<FlakeInput>,
    /// `vars = { ... }` in the outputs let-block (template structure), if any.
    pub vars: Option<Vec<FlakeVar>>,
    /// `channels = [ ... ]` in the outputs let-block, if any.
    pub channels: Option<Vec<String>>,
    pub system: Option<String>,
    pub hosts: Vec<String>,
    /// True when the outputs block has the template's let-structure.
    pub has_template_structure: bool,
    /// Where the lock file was read from (working copy or /etc/nixos).
    pub lock_path: Option<String>,
    pub parse_errors: Vec<ParseErrorInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FlakeOp {
    AddInput { name: String, url: String, follows_nixpkgs: bool },
    RemoveInput { name: String },
    SetInputUrl { name: String, url: String },
    /// Add input nixpkgs-<name> with `url` and append <name> to `channels`.
    AddChannel { name: String, url: String },
    /// Remove the channel entry and its input.
    RemoveChannel { name: String },
    SetVar { name: String, value: String },
    RemoveVar { name: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FlakeEditRequest {
    pub path: String,
    pub version: String,
    pub op: FlakeOp,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConvertToFlakeRequest {
    pub path: String,
    pub hostname: String,
    pub username: Option<String>,
    /// Branch for the main nixpkgs input, e.g. "nixos-unstable".
    pub nixpkgs_ref: String,
}

// ───────────────────────── channels ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum ChannelScope {
    System,
    User,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChannelEntry {
    pub name: String,
    pub url: Option<String>,
    pub version: Option<String>,
    pub scope: ChannelScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChannelsInfo {
    pub system: Vec<ChannelEntry>,
    pub user: Vec<ChannelEntry>,
    /// Channel names available upstream (nixos-unstable, nixos-26.05, ...).
    pub available: Vec<String>,
    pub nixos_version: Option<String>,
    /// Whether the system channel URLs could be read (needs root).
    pub system_urls_known: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum ChannelAction {
    Add,
    Remove,
    Update,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChannelActionRequest {
    pub action: ChannelAction,
    pub scope: ChannelScope,
    pub name: Option<String>,
    pub url: Option<String>,
    /// Required for system scope.
    pub password: Option<String>,
}

// ───────────────────────── package sets ─────────────────────────

/// A package set modules can take packages from: `pkgs` (main) or
/// `pkgs-<channel>` from the flake's extra inputs.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PackageSet {
    /// Module argument name, e.g. "pkgs" or "pkgs-25_11".
    pub name: String,
    /// Human label, e.g. "main · nixos-26.05 (2026-08-20)".
    pub label: String,
    /// Flake input it comes from (None for classic configs).
    pub input: Option<String>,
    /// Installable usable with `nix search`, e.g. "github:nixos/nixpkgs/<rev>".
    pub flake_ref: Option<String>,
    pub is_default: bool,
    /// The i686 variant (`pkgs-<name>-32`).
    pub bits32: bool,
}

// ───────────────────────── disks ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PartitionInfo {
    pub name: String,
    pub path: String,
    /// Bytes.
    #[ts(type = "number")]
    pub size: u64,
    pub fstype: Option<String>,
    pub uuid: Option<String>,
    pub label: Option<String>,
    pub partlabel: Option<String>,
    pub mountpoint: Option<String>,
    /// Mounted as /, /boot, /nix or swap: not something to re-mount.
    pub system: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DiskInfo {
    pub name: String,
    pub path: String,
    #[ts(type = "number")]
    pub size: u64,
    pub model: Option<String>,
    pub transport: Option<String>,
    pub removable: bool,
    pub partitions: Vec<PartitionInfo>,
}

// ───────────────────────── organize ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OrganizeRequest {
    pub path: String,
    pub file: String,
    pub version: String,
    /// Only compute the result, do not write it.
    pub preview: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OrganizeResult {
    pub changed: bool,
    pub groups_created: u32,
    /// The organized source (written to disk unless `preview`).
    pub source: String,
    pub unified_diff: String,
    /// Present when the file was written.
    pub view: Option<NixFileView>,
}

// ───────────────────────── managed secrets ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "lowercase")]
pub enum SecretKind {
    /// Username/password (optionally domain), written in mount.cifs credentials format.
    Smb,
    /// Arbitrary text written as-is.
    Text,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretEntry {
    pub name: String,
    pub kind: SecretKind,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretsDeployInfo {
    /// secrets.age exists in the config folder.
    pub present: bool,
    /// The deployed files match the current secrets.age.
    pub deployed_up_to_date: bool,
    pub deploy_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretsStatus {
    pub exists: bool,
    pub unlocked: bool,
    /// Entries with passwords/content redacted; empty when locked.
    pub entries: Vec<SecretEntry>,
    pub deploy: SecretsDeployInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SecretsRequest {
    pub path: String,
    pub password: Option<String>,
    pub name: Option<String>,
    pub entry: Option<SecretEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ValidateRequest {
    pub path: String,
    pub host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RemoteTestRequest {
    pub ssh_target: String,
    #[serde(default)]
    pub port: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RootKeyRequest {
    pub password: String,
    /// Builder host whose SSH host key should be pre-trusted for root.
    pub builder_host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct KeyAccessRequest {
    /// Name of a saved deploy target.
    pub target: String,
    /// One-time SSH password for the target's user.
    pub password: String,
}

/// Result of the pre-deployment check of a remote target.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RemoteCheck {
    pub reachable: bool,
    pub user: String,
    pub is_root: bool,
    /// The nix daemon on the target trusts the user (None = could not tell).
    pub trusted: Option<bool>,
    /// `sudo -n true` works for the user (None = could not tell).
    pub passwordless_sudo: Option<bool>,
    /// Whether a deployment will need the target's sudo password.
    pub needs_sudo_password: bool,
    pub hostname: Option<String>,
    pub nixos_version: Option<String>,
    /// Where the target's hardware-configuration.nix will come from:
    /// "etc" (/etc/nixos), "generate" (nixos-generate-config on the target),
    /// or "none" (deployment impossible).
    pub hardware_config: Option<String>,
    pub message: Option<String>,
}

// ───────────────────────── git ─────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum GitFileState {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflict,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitFileStatus {
    pub path: String,
    pub state: GitFileState,
    /// Already in the index (git add); the app stages selected files itself anyway.
    pub staged: bool,
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitRemote {
    pub name: String,
    /// Credentials in the URL are redacted.
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitCommit {
    pub hash: String,
    pub subject: String,
    pub date: String,
    pub author: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitInfo {
    pub is_repo: bool,
    pub root: Option<String>,
    pub branch: Option<String>,
    pub detached: bool,
    pub upstream: Option<String>,
    pub ahead: i32,
    pub behind: i32,
    pub remotes: Vec<GitRemote>,
    pub user_name: Option<String>,
    pub user_email: Option<String>,
    pub files: Vec<GitFileStatus>,
    pub last_commit: Option<GitCommit>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitBranch {
    pub name: String,
    pub current: bool,
    pub upstream: Option<String>,
    /// Exists only on the remote; switching to it creates a tracking branch.
    pub remote_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitSuggestion {
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitBranchRequest {
    pub path: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitCommitRequest {
    pub path: String,
    pub message: String,
    /// Files to commit; all changes when null.
    pub paths: Option<Vec<String>>,
    /// Push right after the commit.
    pub push: bool,
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitNetRequest {
    pub path: String,
    /// Password or key passphrase for the remote, only when git asked for one.
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitNetResult {
    pub output: String,
    pub info: GitInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitIdentityRequest {
    pub path: String,
    pub name: String,
    pub email: String,
}

// ───────────────────────── search ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchMatch {
    /// 1-based line.
    pub line: u32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchFile {
    pub rel_path: String,
    /// The path itself contains the query.
    pub name_match: bool,
    pub matches: Vec<SearchMatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SearchResult {
    pub query: String,
    pub files: Vec<SearchFile>,
    /// Some matches were left out (limits reached).
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitRemoteRequest {
    pub path: String,
    pub name: String,
    pub url: String,
}

// ───────────────────────── system page / build status ─────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StoreUsage {
    #[ts(type = "number")]
    pub size: u64,
    #[ts(type = "number")]
    pub used: u64,
    #[ts(type = "number")]
    pub avail: u64,
    pub generations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RollbackRequest {
    pub number: u32,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GcRequest {
    /// 0 = every generation but the current one.
    pub older_than_days: u32,
    pub password: String,
    /// Keep every generation: only free store paths nothing references (plain nix-collect-garbage).
    #[serde(default)]
    pub keep_generations: bool,
}

/// Copy /etc/nixos into a writable working folder.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StartFromSystemRequest {
    /// Destination folder (created; must not exist or be empty).
    pub dest: String,
    pub git_init: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SetHostRequest {
    pub path: String,
    pub host: Option<String>,
}

/// What changed in the working copy since the last successful build of it.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BuildStatus {
    /// RFC 3339 start time of the last successful build/deploy of this folder, if known.
    pub last_build_at: Option<String>,
    pub last_build_title: Option<String>,
    /// .nix files modified after that build (relative paths).
    pub changed_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PopularPackage {
    pub attr: String,
    pub title: String,
    pub description: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitDiscardRequest {
    pub path: String,
    /// Null = everything (git reset --hard).
    pub files: Option<Vec<String>>,
    /// With `files` = null: also delete untracked files and folders (git clean -fd).
    #[serde(default)]
    pub clean_untracked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RemoteSystemInfo {
    pub target: String,
    pub hostname: Option<String>,
    pub generations: Vec<Generation>,
    pub store: Option<StoreUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RemoteSystemRequest {
    pub target: String,
    /// Generation number (rollback) or days (gc).
    pub value: u32,
    /// sudo password on the target; empty when root or passwordless sudo.
    pub password: Option<String>,
    /// gc only: keep every generation (plain nix-collect-garbage).
    #[serde(default)]
    pub keep_generations: bool,
}

/// A NixOS system closure in the local store that no local generation uses
/// (typically a "build here, deploy there" result for another machine).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct OtherSystem {
    pub path: String,
    pub host: String,
    pub nixos_version: String,
    /// Bytes of the whole closure (shared parts included, so sums overlap).
    #[ts(type = "number")]
    pub closure_size: u64,
    /// Bytes used only by this build: not in any local generation's closure
    /// (what a deletion plus garbage collection would free at most).
    #[ts(type = "number")]
    pub exclusive_size: u64,
    /// GC roots still pointing at it (a `result` link, a profile…); empty = can be deleted.
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DeletePathsRequest {
    pub paths: Vec<String>,
    pub password: String,
    /// Run a garbage collection afterwards so the closure's now-unreferenced dependencies go too.
    pub gc_after: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RemoteDeletePathsRequest {
    pub target: String,
    pub paths: Vec<String>,
    pub password: Option<String>,
    pub gc_after: bool,
}
