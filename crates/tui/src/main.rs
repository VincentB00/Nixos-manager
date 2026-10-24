//! Terminal UI over the Nixos-manager core, for machines without a browser
//! (headless servers, a PTTY over SSH). It links the core library directly:
//! no daemon is needed, jobs run in-process and stream into the log view.
//!
//! Screens: pick a folder → workspace → build dialog / validate / system
//! (generations, disk, garbage collection — this machine or a target) /
//! targets (add, edit, check, SSH test, key access, hardware pull) → job log.

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::{execute, terminal};
use futures::StreamExt;
use nixos_manager_core::jobs::{JobHandle, JobManager};
use nixos_manager_core::state::StateStore;
use nixos_manager_core::types::{
    AfterBuild, AppState, BuildStatus, ConfigOverview, DeployRequest, DeployTarget, Generation, HardwareChoice, HardwareStatus, JobDetail, JobEvent, JobStatus,
    KeepConfiguration, LocationSuggestion, LogStream, OtherSystem, RebuildAction, RemoteCheck, StoreUsage,
};
use nixos_manager_core::{deploy, discover, fsbrowse, hardware, nix, remote, system, validate};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

// ───────────────────────── messages from background work ─────────────────────────

/// Generations / disk of one machine (None = this one).
struct SysInfo {
    machine: Option<String>,
    hostname: Option<String>,
    generations: Vec<Generation>,
    store: Option<StoreUsage>,
}

enum Msg {
    State(AppState),
    Suggestions(Vec<LocationSuggestion>),
    Opened(Result<(PathBuf, ConfigOverview, Option<BuildStatus>, HardwareStatus), String>),
    Check(String, RemoteCheck),
    SystemInfo(Result<SysInfo, String>),
    OtherSystems(Option<String>, Result<Vec<OtherSystem>, String>),
    Job(JobEvent),
    OpenJob(String),
    JobDetail(Box<JobDetail>),
    Info(String),
    Error(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Pick,
    Workspace,
    Build,
    System,
    Targets,
    TargetEdit,
    Job,
    Jobs,
}

// ───────────────────────── build form ─────────────────────────

/// One row of the build form.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    DeployTo,
    Mode,
    BuildOn,
    Host,
    Action,
    Upgrade,
    SkipSync,
    Hardware,
    After,
    AfterLocal,
    AfterAlways,
    LocalPassword,
    TargetPassword,
    Start,
}

struct BuildForm {
    cursor: usize,
    target_idx: usize, // 0 = this machine
    on_target: bool,
    build_on_idx: usize, // 0 = compile here (only when deploying to this machine)
    host_idx: usize,
    action: RebuildAction,
    upgrade: bool,
    skip_sync: bool,
    workspace_hw: bool,
    after: Option<AfterBuild>,
    after_local: Option<AfterBuild>,
    after_always: bool,
    local_password: String,
    target_password: String,
}

impl BuildForm {
    fn new() -> Self {
        Self {
            cursor: 0, target_idx: 0, on_target: false, build_on_idx: 0, host_idx: 0, action: RebuildAction::Switch, upgrade: false, skip_sync: false, workspace_hw: false,
            after: None, after_local: None, after_always: false, local_password: String::new(), target_password: String::new(),
        }
    }
    fn fields(&self, has_targets: bool, hosts: usize) -> Vec<Field> {
        let mut f = Vec::new();
        let remote = self.target_idx > 0;
        if has_targets {
            f.push(Field::DeployTo);
        }
        if remote {
            f.push(Field::Mode);
        } else if has_targets {
            f.push(Field::BuildOn);
        }
        if hosts > 1 {
            f.push(Field::Host);
        }
        f.push(Field::Action);
        f.push(Field::Upgrade);
        if !remote {
            f.push(Field::SkipSync);
        } else {
            f.push(Field::Hardware);
        }
        f.push(Field::After);
        if remote {
            f.push(Field::AfterLocal);
        }
        if after_set(self.after) || (remote && after_set(self.after_local)) {
            f.push(Field::AfterAlways);
        }
        if !remote && !deploy::running_as_root() {
            f.push(Field::LocalPassword);
        }
        if remote || self.build_on_idx > 0 {
            f.push(Field::TargetPassword);
        }
        f.push(Field::Start);
        f
    }
}

fn after_set(a: Option<AfterBuild>) -> bool {
    matches!(a, Some(AfterBuild::Reboot) | Some(AfterBuild::Poweroff))
}

fn cycle_after(a: Option<AfterBuild>) -> Option<AfterBuild> {
    match a {
        None | Some(AfterBuild::None) => Some(AfterBuild::Reboot),
        Some(AfterBuild::Reboot) => Some(AfterBuild::Poweroff),
        Some(AfterBuild::Poweroff) => None,
    }
}

fn after_label(a: Option<AfterBuild>) -> &'static str {
    match a {
        Some(AfterBuild::Reboot) => "reboot",
        Some(AfterBuild::Poweroff) => "power off",
        _ => "stay on",
    }
}

// ───────────────────────── target form ─────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum TField {
    Name,
    Host,
    User,
    Port,
    FlakeHost,
    Sudo,
    Save,
}

const TFIELDS: [TField; 7] = [TField::Name, TField::Host, TField::User, TField::Port, TField::FlakeHost, TField::Sudo, TField::Save];

struct TargetForm {
    cursor: usize,
    edit_idx: Option<usize>,
    name: String,
    host: String,
    user: String,
    port: String,
    flake_host: String,
    sudo: bool,
}

impl TargetForm {
    fn blank() -> Self {
        Self { cursor: 0, edit_idx: None, name: String::new(), host: String::new(), user: String::new(), port: String::new(), flake_host: String::new(), sudo: true }
    }
    fn from_target(i: usize, t: &DeployTarget) -> Self {
        let (user, host) = match t.ssh_target.split_once('@') {
            Some((u, h)) => (u.to_string(), h.to_string()),
            None => (String::new(), t.ssh_target.clone()),
        };
        Self { cursor: 0, edit_idx: Some(i), name: t.name.clone(), host, user, port: t.port.map(|p| p.to_string()).unwrap_or_default(), flake_host: t.flake_host.clone().unwrap_or_default(), sudo: t.use_remote_sudo }
    }
    fn text_mut(&mut self, f: TField) -> Option<&mut String> {
        match f {
            TField::Name => Some(&mut self.name),
            TField::Host => Some(&mut self.host),
            TField::User => Some(&mut self.user),
            TField::Port => Some(&mut self.port),
            TField::FlakeHost => Some(&mut self.flake_host),
            _ => None,
        }
    }
    /// Same rules as the web dialog: user@host, port only when not 22, root never needs sudo.
    fn build(&self, previous: Option<&DeployTarget>) -> std::result::Result<DeployTarget, String> {
        let name = self.name.trim().to_string();
        let host = self.host.trim().to_string();
        let user = self.user.trim().to_string();
        if name.is_empty() {
            return Err("the target needs a name".into());
        }
        if host.is_empty() {
            return Err("the target needs a host name or IP".into());
        }
        let port = match self.port.trim() {
            "" | "22" => None,
            p => Some(p.parse::<u16>().ok().filter(|p| *p > 0).ok_or_else(|| format!("'{p}' is not a valid port"))?),
        };
        Ok(DeployTarget {
            name,
            ssh_target: if user.is_empty() { host } else { format!("{user}@{host}") },
            flake_host: Some(self.flake_host.trim().to_string()).filter(|s| !s.is_empty()),
            build_host: previous.and_then(|p| p.build_host.clone()),
            use_remote_sudo: if user == "root" { false } else { self.sudo },
            port,
        })
    }
}

// ───────────────────────── confirmation / password popup ─────────────────────────

enum PromptAction {
    Rollback(u32),
    Gc { days: u32, keep: bool },
    DeletePaths(Vec<String>),
    SetupKey(DeployTarget),
    RootKey(Option<String>),
    FreeMemory,
    DeleteTarget(String),
}

struct Prompt {
    title: String,
    lines: Vec<String>,
    action: PromptAction,
    password: String,
    /// Show the password field at all (false for plain confirmations).
    ask_password: bool,
    /// Refuse an empty password (local sudo); remote root / passwordless sudo may leave it empty.
    require_password: bool,
}

// ───────────────────────── system view (generations, disk, other builds) ─────────────────────────

struct SystemView {
    machine: Option<DeployTarget>,
    hostname: Option<String>,
    generations: Vec<Generation>,
    store: Option<StoreUsage>,
    others: Vec<OtherSystem>,
    gen_list: ListState,
    other_list: ListState,
    focus_other: bool,
    loading: bool,
    others_loading: bool,
    error: Option<String>,
    others_error: Option<String>,
}

impl SystemView {
    fn new(machine: Option<DeployTarget>) -> Self {
        Self {
            machine, hostname: None, generations: vec![], store: None, others: vec![], gen_list: ListState::default().with_selected(Some(0)), other_list: ListState::default().with_selected(Some(0)),
            focus_other: false, loading: true, others_loading: true, error: None, others_error: None,
        }
    }
    fn machine_name(&self) -> Option<String> {
        self.machine.as_ref().map(|t| t.name.clone())
    }
}

// ───────────────────────── app ─────────────────────────

struct App {
    screen: Screen,
    store: Arc<StateStore>,
    jobs: JobManager,
    tx: mpsc::UnboundedSender<Msg>,
    status: Option<String>,
    prompt: Option<Prompt>,
    // pick
    state: Option<AppState>,
    suggestions: Vec<LocationSuggestion>,
    pick_list: ListState,
    pick_input: String,
    pick_input_focus: bool,
    // workspace
    root: Option<PathBuf>,
    overview: Option<ConfigOverview>,
    build_status: Option<BuildStatus>,
    hw: Option<HardwareStatus>,
    host: Option<String>,
    targets: Vec<DeployTarget>,
    // remote checks, by target name
    checks: HashMap<String, RemoteCheck>,
    checking: HashSet<String>,
    // targets screen
    target_list: ListState,
    tform: TargetForm,
    // build
    form: BuildForm,
    // system
    sys: SystemView,
    // jobs
    job_id: Option<String>,
    job: Option<JobDetail>,
    job_scroll: usize,
    job_follow: bool,
    job_ids: Vec<String>,
    jobs_list: ListState,
    job_back: Screen,
}

impl App {
    fn new(store: Arc<StateStore>, jobs: JobManager, tx: mpsc::UnboundedSender<Msg>) -> Self {
        Self {
            screen: Screen::Pick, store, jobs, tx, status: None, prompt: None, state: None, suggestions: vec![], pick_list: ListState::default().with_selected(Some(0)), pick_input: String::new(), pick_input_focus: false,
            root: None, overview: None, build_status: None, hw: None, host: None, targets: vec![], checks: HashMap::new(), checking: HashSet::new(),
            target_list: ListState::default().with_selected(Some(0)), tform: TargetForm::blank(), form: BuildForm::new(), sys: SystemView::new(None),
            job_id: None, job: None, job_scroll: 0, job_follow: true, job_ids: vec![], jobs_list: ListState::default().with_selected(Some(0)), job_back: Screen::Workspace,
        }
    }

    fn pick_entries(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = Vec::new();
        if let Some(s) = &self.state {
            for r in &s.recent_locations {
                v.push((r.path.clone(), format!("recent · host {}", r.host.clone().unwrap_or_else(|| "-".into()))));
            }
        }
        for s in &self.suggestions {
            if !v.iter().any(|(p, _)| p == &s.path) {
                v.push((s.path.clone(), format!("{:?} · {}", s.kind, s.reason).to_lowercase()));
            }
        }
        v
    }

    fn refresh_state(&self) {
        let store = self.store.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let st = store.get().await;
            let _ = tx.send(Msg::State(st));
            let sug = tokio::task::spawn_blocking(fsbrowse::suggestions).await.unwrap_or_default();
            let _ = tx.send(Msg::Suggestions(sug));
        });
    }

    fn open(&mut self, path: String) {
        let tx = self.tx.clone();
        let store = self.store.clone();
        self.status = Some(format!("Opening {path}…"));
        tokio::spawn(async move {
            let res: Result<(PathBuf, ConfigOverview, Option<BuildStatus>, HardwareStatus), String> = async {
                let root = fsbrowse::config_root(&path).map_err(|e| e.to_string())?;
                // same housekeeping as the web UI on open
                let pull = store.get().await.hardware_pulls.get(&root.to_string_lossy().to_string()).cloned();
                let _ = hardware::sync_from_system(&root, pull.as_ref());
                let _ = nixos_manager_core::flake::remove_workspace_lock(&root);
                let hw = hardware::status(&root, pull.as_ref());
                let ov = discover::overview(&root).await.map_err(|e| e.to_string())?;
                store.touch_location(&root.to_string_lossy(), ov.location.hosts.first().cloned()).await.map_err(|e| e.to_string())?;
                let r2 = root.clone();
                let bs = tokio::task::spawn_blocking(move || JobManager::build_status(&r2)).await.ok();
                Ok((root, ov, bs, hw))
            }
            .await;
            let _ = tx.send(Msg::Opened(res));
        });
    }

    /// Create a job, hand it to `make`, and open its log.
    fn run_job<Fut>(&self, kind: &'static str, title: String, make: impl FnOnce(JobHandle) -> Fut + Send + 'static)
    where
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let jobs = self.jobs.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let job = jobs.create(kind, &title).await;
            let id = job.id.clone();
            tokio::spawn(make(job));
            let _ = tx.send(Msg::OpenJob(id));
        });
    }

    // ── remote checks ──

    fn check_target(&mut self, name: &str) {
        let Some(t) = self.targets.iter().find(|t| t.name == name).cloned() else { return };
        if !self.checking.insert(t.name.clone()) {
            return;
        }
        self.checks.remove(&t.name);
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let c = remote::check(&t).await;
            let _ = tx.send(Msg::Check(t.name.clone(), c));
        });
    }

    /// Whether root actions on `t` need its sudo password: Some(false) = root or
    /// passwordless sudo, None = not checked yet.
    fn remote_needs_password(&self, t: &DeployTarget) -> Option<bool> {
        self.checks.get(&t.name).map(|c| !c.is_root && c.passwordless_sudo != Some(true))
    }

    fn selected_target(&self) -> Option<DeployTarget> {
        self.target_list.selected().and_then(|i| self.targets.get(i)).cloned()
    }

    // ── build ──

    fn form_target(&self) -> Option<DeployTarget> {
        if self.form.target_idx > 0 { self.targets.get(self.form.target_idx - 1).cloned() } else { None }
    }
    fn form_builder(&self) -> Option<DeployTarget> {
        if self.form.target_idx == 0 && self.form.build_on_idx > 0 { self.targets.get(self.form.build_on_idx - 1).cloned() } else { None }
    }

    fn start_build(&mut self) {
        let Some(root) = self.root.clone() else { return };
        let target = self.form_target();
        let builder = self.form_builder();
        let hosts = self.overview.as_ref().map(|o| o.location.hosts.clone()).unwrap_or_default();
        let host = hosts.get(self.form.host_idx).cloned().or(self.host.clone());
        if target.is_none() && self.form.local_password.is_empty() && !deploy::running_as_root() {
            self.status = Some("Enter your sudo password first.".into());
            return;
        }
        if let Some(t) = target.as_ref().or(builder.as_ref()) {
            if self.remote_needs_password(t) == Some(true) && self.form.target_password.is_empty() {
                self.status = Some(format!("{} needs its sudo password (type it in the field above).", t.name));
                return;
            }
            if let Some(c) = self.checks.get(&t.name) {
                if !c.reachable {
                    self.status = Some(format!("Cannot reach {}: {}", t.ssh_target, c.message.clone().unwrap_or_default()));
                    return;
                }
                if target.is_some() && !self.form.on_target && !self.form.workspace_hw && c.hardware_config.as_deref() == Some("none") {
                    self.status = Some("The target has no hardware-configuration.nix: switch Hardware to the working copy's file.".into());
                    return;
                }
            }
        }
        let any_after = after_set(self.form.after) || (target.is_some() && after_set(self.form.after_local));
        let req = DeployRequest {
            path: root.to_string_lossy().to_string(),
            host,
            action: self.form.action,
            upgrade: self.form.upgrade,
            keep_configuration: Some(KeepConfiguration::Local),
            hardware: Some(if target.is_some() && self.form.workspace_hw { HardwareChoice::Workspace } else { HardwareChoice::System }),
            skip_sync: target.is_none() && self.form.skip_sync,
            password: self.form.local_password.clone(),
            secrets_password: None,
            target: target.clone(),
            target_sudo_password: if (target.is_some() || builder.is_some()) && !self.form.target_password.is_empty() { Some(self.form.target_password.clone()) } else { None },
            build_host: builder,
            after: self.form.after,
            after_even_on_failure: any_after && self.form.after_always,
            after_local: if target.is_some() { self.form.after_local } else { None },
            on_target: target.is_some() && self.form.on_target,
            allow_unfree_env: target.is_some() && self.form.on_target,
        };
        let title = match &target {
            Some(t) if req.on_target => format!("nixos-rebuild {:?} on {} ({})", req.action, t.name, req.path),
            Some(t) => format!("nixos-rebuild {:?} → {} ({})", req.action, t.name, req.path),
            None => format!("nixos-rebuild {:?} ({})", req.action, req.path),
        };
        self.run_job("deploy", title, move |job| deploy::run(job, req));
        self.form.local_password.clear();
        self.form.target_password.clear();
    }

    fn start_validate(&mut self) {
        let Some(root) = self.root.clone() else { return };
        let host = self.host.clone();
        let title = format!("Validate {}", root.display());
        self.run_job("validate", title, move |job| validate::run(job, root, host));
    }

    // ── system (this machine or a target) ──

    fn open_system(&mut self, machine: Option<DeployTarget>) {
        if let Some(t) = &machine {
            if !self.checks.contains_key(&t.name) {
                let n = t.name.clone();
                self.check_target(&n);
            }
        }
        self.sys = SystemView::new(machine);
        self.screen = Screen::System;
        self.load_system();
    }

    fn load_system(&mut self) {
        self.sys.loading = true;
        self.sys.others_loading = true;
        self.sys.error = None;
        self.sys.others_error = None;
        let tx = self.tx.clone();
        let machine = self.sys.machine.clone();
        let name = self.sys.machine_name();
        let (m2, n2, tx2) = (machine.clone(), name.clone(), tx.clone());
        tokio::spawn(async move {
            let res = match &machine {
                None => {
                    let gens = nix::generations().await.map_err(|e| e.to_string());
                    let store = system::store_usage().await.ok();
                    gens.map(|g| SysInfo { machine: None, hostname: None, generations: g, store })
                }
                Some(t) => system::remote_info(t).await.map(|i| SysInfo { machine: name.clone(), hostname: i.hostname, generations: i.generations, store: i.store }).map_err(|e| e.to_string()),
            };
            let _ = tx.send(Msg::SystemInfo(res));
        });
        tokio::spawn(async move {
            let res = match &m2 {
                None => system::other_systems().await.map_err(|e| e.to_string()),
                Some(t) => system::remote_other_systems(t).await.map_err(|e| e.to_string()),
            };
            let _ = tx2.send(Msg::OtherSystems(n2, res));
        });
    }

    /// Open the password popup for a root action on the system screen's machine.
    fn ask_system(&mut self, title: &str, mut lines: Vec<String>, action: PromptAction) {
        let (ask, require, lines) = match &self.sys.machine {
            None => (!deploy::running_as_root(), !deploy::running_as_root(), lines),
            Some(t) => match self.remote_needs_password(t) {
                Some(false) => (false, false, lines),
                Some(true) => (true, true, lines),
                None => {
                    lines.push(format!("Leave the password empty if {} is root or has passwordless sudo.", t.ssh_target));
                    (true, false, lines)
                }
            },
        };
        self.prompt = Some(Prompt { title: title.to_string(), lines, action, password: String::new(), ask_password: ask, require_password: require });
    }

    fn run_prompt(&mut self, p: Prompt) {
        let pw = p.password;
        let machine = self.sys.machine.clone();
        let mname = machine.as_ref().map(|t| t.name.clone()).unwrap_or_else(|| "this machine".into());
        match p.action {
            PromptAction::Rollback(n) => match machine {
                None => self.run_job("system", format!("Roll back to generation {n}"), move |job| system::rollback(job, n, pw)),
                Some(t) => self.run_job("system", format!("Roll back {} to generation {n}", t.name), move |job| system::remote_rollback(job, t, n, pw)),
            },
            PromptAction::Gc { days, keep } => {
                let what = if keep { "keep all generations".to_string() } else if days == 0 { "delete all old generations".into() } else { format!("older than {days} days") };
                let title = if machine.is_some() { format!("Collect garbage on {mname} ({what})") } else { format!("Collect garbage ({what})") };
                match machine {
                    None => self.run_job("system", title, move |job| system::collect_garbage(job, days, pw, keep)),
                    Some(t) => self.run_job("system", title, move |job| system::remote_gc(job, t, days, pw, keep)),
                }
            }
            PromptAction::DeletePaths(paths) => {
                let title = format!("Delete {} build(s) for other machines on {mname}", paths.len());
                match machine {
                    None => self.run_job("system", title, move |job| system::delete_paths(job, paths, pw, true)),
                    Some(t) => self.run_job("system", title, move |job| system::remote_delete_paths(job, t, paths, pw, true)),
                }
            }
            PromptAction::SetupKey(t) => {
                let title = format!("Set up key access to {}", t.name);
                self.run_job("setup-key", title, move |job| remote::setup_key_access(job, t, pw));
            }
            PromptAction::RootKey(host) => {
                self.run_job("root-key", "Root SSH key for remote builders".into(), move |job| remote::root_builder_key(job, pw, host));
            }
            PromptAction::FreeMemory => self.run_free_memory(pw),
            PromptAction::DeleteTarget(name) => {
                let next: Vec<DeployTarget> = self.targets.iter().filter(|t| t.name != name).cloned().collect();
                self.save_targets(next, format!("Removed target {name}."));
            }
        }
    }

    /// Vincent's clear-sessions.sh: kill every other login session and drop caches.
    fn run_free_memory(&mut self, pw: String) {
        let my_session = std::env::var("XDG_SESSION_ID").ok().filter(|s| !s.is_empty()).or_else(|| {
            std::process::Command::new("loginctl").args(["show-session", "self", "-p", "Id", "--value"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|s| !s.is_empty())
        }).unwrap_or_default();
        let my_user = std::env::var("USER").unwrap_or_else(|_| "root".into());
        self.run_job("system", "Free memory: terminate other sessions, drop caches".into(), move |job| system::free_memory(job, pw, my_session, my_user));
    }

    // ── targets ──

    fn save_targets(&mut self, next: Vec<DeployTarget>, done: String) {
        let store = self.store.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            match store.set_targets(next).await {
                Ok(st) => {
                    let _ = tx.send(Msg::State(st));
                    let _ = tx.send(Msg::Info(done));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Error(format!("cannot save the targets: {e}")));
                }
            }
        });
    }

    fn save_target_form(&mut self) {
        let prev = self.tform.edit_idx.and_then(|i| self.targets.get(i)).cloned();
        let t = match self.tform.build(prev.as_ref()) {
            Ok(t) => t,
            Err(e) => {
                self.status = Some(e);
                return;
            }
        };
        if self.targets.iter().enumerate().any(|(i, x)| x.name == t.name && Some(i) != self.tform.edit_idx) {
            self.status = Some(format!("A target named {} already exists.", t.name));
            return;
        }
        let mut next = self.targets.clone();
        match self.tform.edit_idx {
            Some(i) if i < next.len() => next[i] = t.clone(),
            _ => next.push(t.clone()),
        }
        if let Some(p) = &prev {
            self.checks.remove(&p.name);
        }
        self.checks.remove(&t.name);
        self.screen = Screen::Targets;
        self.save_targets(next, format!("Saved target {}.", t.name));
    }

    fn test_connection(&mut self, t: DeployTarget) {
        let title = format!("Test connection to {}", t.ssh_target);
        self.run_job("remote-test", title, move |job| remote::test_connection(job, t.ssh_target, t.port));
    }

    /// Copy the target's /etc/nixos/hardware-configuration.nix over the working copy's file and remember it.
    fn pull_hardware(&mut self, t: DeployTarget) {
        let Some(root) = self.root.clone() else {
            self.status = Some("Open a working folder first.".into());
            return;
        };
        let store = self.store.clone();
        let tx = self.tx.clone();
        self.status = Some(format!("Fetching hardware-configuration.nix from {}…", t.name));
        tokio::spawn(async move {
            let res: std::result::Result<String, String> = async {
                let (source, text) = remote::fetch_target_hardware(&t).await?;
                let (rel, pull) = hardware::install_pulled(&root, &t.name, &text).map_err(|e| format!("cannot write the file: {e}"))?;
                store.set_hardware_pull(&root.to_string_lossy(), pull).await.map_err(|e| e.to_string())?;
                Ok(format!("Copied {} from {} ({source}, {} bytes) into the working copy.", rel.display(), t.name, text.len()))
            }
            .await;
            let _ = tx.send(match res {
                Ok(m) => Msg::Info(m),
                Err(e) => Msg::Error(e),
            });
        });
    }

    // ── jobs ──

    fn open_job(&mut self, id: String) {
        self.job_id = Some(id.clone());
        self.job = None;
        self.job_scroll = 0;
        self.job_follow = true;
        if !self.job_ids.contains(&id) {
            self.job_ids.insert(0, id.clone());
        }
        if !matches!(self.screen, Screen::Job | Screen::Jobs) {
            self.job_back = self.screen;
        }
        self.screen = Screen::Job;
        self.refresh_job();
    }

    fn refresh_job(&mut self) {
        let Some(id) = self.job_id.clone() else { return };
        let jobs = self.jobs.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Some(d) = jobs.get(&id).await {
                let _ = tx.send(Msg::JobDetail(Box::new(d)));
            }
        });
    }
}

fn human(bytes: u64) -> String {
    const U: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{bytes} B") } else { format!("{v:.1} {}", U[i]) }
}

fn yes_no(b: Option<bool>) -> &'static str {
    match b {
        Some(true) => "yes",
        Some(false) => "no",
        None => "?",
    }
}

// ───────────────────────── key handling ─────────────────────────

fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return false;
    }
    if app.prompt.is_some() {
        key_prompt(app, key);
        return true;
    }
    match app.screen {
        Screen::Pick => key_pick(app, key),
        Screen::Workspace => key_workspace(app, key),
        Screen::Build => key_build(app, key),
        Screen::System => key_system(app, key),
        Screen::Targets => key_targets(app, key),
        Screen::TargetEdit => key_target_edit(app, key),
        Screen::Job => key_job(app, key),
        Screen::Jobs => key_jobs(app, key),
    }
}

fn key_prompt(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.prompt = None,
        KeyCode::Enter => {
            let Some(p) = app.prompt.take() else { return };
            if p.ask_password && p.require_password && p.password.is_empty() {
                app.status = Some("Enter your sudo password first.".into());
                app.prompt = Some(p);
                return;
            }
            app.run_prompt(p);
        }
        KeyCode::Backspace => {
            if let Some(p) = app.prompt.as_mut() {
                p.password.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Some(p) = app.prompt.as_mut() {
                if p.ask_password {
                    p.password.push(c);
                }
            }
        }
        _ => {}
    }
}

fn key_pick(app: &mut App, key: KeyEvent) -> bool {
    let n = app.pick_entries().len();
    match key.code {
        KeyCode::Char('q') if !app.pick_input_focus => return false,
        KeyCode::Char('t') if !app.pick_input_focus => {
            app.target_list.select(Some(0));
            app.screen = Screen::Targets;
        }
        KeyCode::Tab => app.pick_input_focus = !app.pick_input_focus,
        KeyCode::Esc => app.pick_input_focus = false,
        KeyCode::Enter => {
            if app.pick_input_focus && !app.pick_input.trim().is_empty() {
                let p = app.pick_input.trim().to_string();
                app.open(p);
            } else if let Some(i) = app.pick_list.selected() {
                if let Some((p, _)) = app.pick_entries().get(i) {
                    let p = p.clone();
                    app.open(p);
                }
            }
        }
        KeyCode::Up if !app.pick_input_focus => {
            let i = app.pick_list.selected().unwrap_or(0);
            app.pick_list.select(Some(i.saturating_sub(1)));
        }
        KeyCode::Down if !app.pick_input_focus => {
            let i = app.pick_list.selected().unwrap_or(0);
            if n > 0 {
                app.pick_list.select(Some((i + 1).min(n - 1)));
            }
        }
        KeyCode::Backspace if app.pick_input_focus => {
            app.pick_input.pop();
        }
        KeyCode::Char(c) if app.pick_input_focus => app.pick_input.push(c),
        KeyCode::Char('/') => app.pick_input_focus = true,
        _ => {}
    }
    true
}

fn key_workspace(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('m') => {
            app.prompt = Some(Prompt {
                title: "Free memory".into(),
                lines: vec![
                    "Logs out every other session (the desktop loses unsaved work), stops".into(),
                    "lingering user managers and drops the page cache. This session stays.".into(),
                    "Run it from the TTY or SSH session you will build from.".into(),
                ],
                action: PromptAction::FreeMemory,
                password: String::new(),
                ask_password: !deploy::running_as_root(),
                require_password: !deploy::running_as_root(),
            });
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            app.screen = Screen::Pick;
            app.refresh_state();
        }
        KeyCode::Char('b') => {
            app.form = BuildForm::new();
            if let Some(ov) = &app.overview {
                app.form.host_idx = ov.location.hosts.iter().position(|h| Some(h) == app.host.as_ref()).unwrap_or(0);
            }
            app.screen = Screen::Build;
        }
        KeyCode::Char('v') => app.start_validate(),
        KeyCode::Char('g') => app.open_system(None),
        KeyCode::Char('t') => {
            app.target_list.select(Some(0));
            app.screen = Screen::Targets;
        }
        KeyCode::Char('j') => app.screen = Screen::Jobs,
        KeyCode::Char('h') => {
            if let Some(ov) = &app.overview {
                let hosts = &ov.location.hosts;
                if !hosts.is_empty() {
                    let i = hosts.iter().position(|h| Some(h) == app.host.as_ref()).map(|i| (i + 1) % hosts.len()).unwrap_or(0);
                    app.host = Some(hosts[i].clone());
                    let (store, root, host) = (app.store.clone(), app.root.clone(), app.host.clone());
                    tokio::spawn(async move {
                        if let Some(r) = root {
                            let _ = store.touch_location(&r.to_string_lossy(), host).await;
                        }
                    });
                }
            }
        }
        KeyCode::Char('r') => {
            if let Some(r) = app.root.clone() {
                app.open(r.to_string_lossy().to_string());
            }
        }
        _ => {}
    }
    true
}

fn key_build(app: &mut App, key: KeyEvent) -> bool {
    let hosts = app.overview.as_ref().map(|o| o.location.hosts.len()).unwrap_or(0);
    let fields = app.form.fields(!app.targets.is_empty(), hosts);
    let cur = fields.get(app.form.cursor.min(fields.len().saturating_sub(1))).copied().unwrap_or(Field::Start);
    let typing = matches!(cur, Field::LocalPassword | Field::TargetPassword);
    let n_t = app.targets.len() + 1;
    match key.code {
        KeyCode::Esc => app.screen = Screen::Workspace,
        KeyCode::Up => app.form.cursor = app.form.cursor.saturating_sub(1),
        KeyCode::Down | KeyCode::Tab => app.form.cursor = (app.form.cursor + 1).min(fields.len() - 1),
        KeyCode::Enter if cur == Field::Start => app.start_build(),
        KeyCode::Enter | KeyCode::Right | KeyCode::Left | KeyCode::Char(' ') if !typing => {
            let fwd = key.code != KeyCode::Left;
            match cur {
                Field::DeployTo => {
                    app.form.target_idx = if fwd { (app.form.target_idx + 1) % n_t } else { (app.form.target_idx + n_t - 1) % n_t };
                    if let Some(t) = app.form_target() {
                        app.check_target(&t.name);
                    }
                }
                Field::BuildOn => {
                    app.form.build_on_idx = if fwd { (app.form.build_on_idx + 1) % n_t } else { (app.form.build_on_idx + n_t - 1) % n_t };
                    if let Some(t) = app.form_builder() {
                        app.check_target(&t.name);
                    }
                }
                Field::Mode => app.form.on_target = !app.form.on_target,
                Field::Host => {
                    if hosts > 0 {
                        app.form.host_idx = if fwd { (app.form.host_idx + 1) % hosts } else { (app.form.host_idx + hosts - 1) % hosts };
                    }
                }
                Field::Action => {
                    let all = [RebuildAction::Switch, RebuildAction::Boot, RebuildAction::Test, RebuildAction::Build, RebuildAction::DryBuild];
                    let i = all.iter().position(|a| *a == app.form.action).unwrap_or(0);
                    app.form.action = all[if fwd { (i + 1) % all.len() } else { (i + all.len() - 1) % all.len() }];
                }
                Field::Upgrade => app.form.upgrade = !app.form.upgrade,
                Field::SkipSync => app.form.skip_sync = !app.form.skip_sync,
                Field::Hardware => app.form.workspace_hw = !app.form.workspace_hw,
                Field::After => app.form.after = cycle_after(app.form.after),
                Field::AfterLocal => app.form.after_local = cycle_after(app.form.after_local),
                Field::AfterAlways => app.form.after_always = !app.form.after_always,
                _ => {}
            }
        }
        KeyCode::Enter if typing => app.form.cursor = (app.form.cursor + 1).min(fields.len() - 1),
        KeyCode::Backspace if typing => {
            if cur == Field::LocalPassword { app.form.local_password.pop(); } else { app.form.target_password.pop(); }
        }
        KeyCode::Char(c) if typing => {
            if cur == Field::LocalPassword { app.form.local_password.push(c); } else { app.form.target_password.push(c); }
        }
        _ => {}
    }
    true
}

fn key_system(app: &mut App, key: KeyEvent) -> bool {
    let machine = app.sys.machine_name().unwrap_or_else(|| "this machine".into());
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.screen = if app.sys.machine.is_some() { Screen::Targets } else { Screen::Workspace },
        KeyCode::Tab => {
            if !app.sys.others.is_empty() {
                app.sys.focus_other = !app.sys.focus_other;
            }
        }
        KeyCode::Up | KeyCode::Down => {
            let (list, n) = if app.sys.focus_other { (&mut app.sys.other_list, app.sys.others.len()) } else { (&mut app.sys.gen_list, app.sys.generations.len()) };
            let i = list.selected().unwrap_or(0);
            if key.code == KeyCode::Up {
                list.select(Some(i.saturating_sub(1)));
            } else if n > 0 {
                list.select(Some((i + 1).min(n - 1)));
            }
        }
        KeyCode::Enter if app.sys.focus_other => {
            if let Some(o) = app.sys.other_list.selected().and_then(|i| app.sys.others.get(i)).cloned() {
                if !o.roots.is_empty() {
                    app.status = Some(format!("Still referenced by {} — remove that first.", o.roots.join(", ")));
                } else {
                    let lines = vec![
                        format!("Delete the build for {} (NixOS {}) from the store of {machine}?", o.host, o.nixos_version),
                        format!("Frees up to {} (only this build), then garbage collection runs.", human(o.exclusive_size)),
                    ];
                    app.ask_system("Delete build", lines, PromptAction::DeletePaths(vec![o.path.clone()]));
                }
            }
        }
        KeyCode::Enter => {
            if let Some(g) = app.sys.gen_list.selected().and_then(|i| app.sys.generations.get(i)).cloned() {
                if g.current {
                    app.status = Some("That generation is already the current one.".into());
                } else {
                    let lines = vec![format!("Roll {machine} back to generation #{}? It switches to it now and at boot.", g.number)];
                    app.ask_system("Roll back", lines, PromptAction::Rollback(g.number));
                }
            }
        }
        KeyCode::Char('c') => app.ask_system("Collect garbage", vec![format!("nix-collect-garbage on {machine} without deleting any generation: frees packages nothing references any more.")], PromptAction::Gc { days: 0, keep: true }),
        KeyCode::Char('o') => app.ask_system("Collect garbage", vec![format!("nix-collect-garbage --delete-older-than 30d on {machine}: removes generations older than 30 days, then frees their packages.")], PromptAction::Gc { days: 30, keep: false }),
        KeyCode::Char('D') => app.ask_system("Collect garbage", vec![format!("nix-collect-garbage -d on {machine}: deletes every generation but the current one, then frees their packages. You cannot roll back afterwards.")], PromptAction::Gc { days: 0, keep: false }),
        KeyCode::Char('r') => app.load_system(),
        _ => {}
    }
    true
}

fn key_targets(app: &mut App, key: KeyEvent) -> bool {
    let n = app.targets.len();
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.screen = if app.root.is_some() { Screen::Workspace } else { Screen::Pick },
        KeyCode::Up => {
            let i = app.target_list.selected().unwrap_or(0);
            app.target_list.select(Some(i.saturating_sub(1)));
        }
        KeyCode::Down => {
            let i = app.target_list.selected().unwrap_or(0);
            if n > 0 {
                app.target_list.select(Some((i + 1).min(n - 1)));
            }
        }
        KeyCode::Char('a') => {
            app.tform = TargetForm::blank();
            app.screen = Screen::TargetEdit;
        }
        KeyCode::Char('e') => {
            if let Some(i) = app.target_list.selected() {
                if let Some(t) = app.targets.get(i) {
                    app.tform = TargetForm::from_target(i, t);
                    app.screen = Screen::TargetEdit;
                }
            }
        }
        KeyCode::Char('d') => {
            if let Some(t) = app.selected_target() {
                app.prompt = Some(Prompt {
                    title: "Remove target".into(),
                    lines: vec![format!("Remove {} ({}) from the deploy targets? Nothing changes on the machine itself.", t.name, t.ssh_target)],
                    action: PromptAction::DeleteTarget(t.name.clone()),
                    password: String::new(),
                    ask_password: false,
                    require_password: false,
                });
            }
        }
        KeyCode::Char('c') | KeyCode::Enter => {
            if let Some(t) = app.selected_target() {
                app.check_target(&t.name);
            }
        }
        KeyCode::Char('t') => {
            if let Some(t) = app.selected_target() {
                app.test_connection(t);
            }
        }
        KeyCode::Char('k') => {
            if let Some(t) = app.selected_target() {
                app.prompt = Some(Prompt {
                    title: format!("Key access to {}", t.name),
                    lines: vec![
                        format!("Installs this app's SSH key in the authorized_keys of {} using its password once;", t.ssh_target),
                        "later connections need no password. The password is only sent to ssh.".into(),
                    ],
                    action: PromptAction::SetupKey(t.clone()),
                    password: String::new(),
                    ask_password: true,
                    require_password: true,
                });
            }
        }
        KeyCode::Char('p') => {
            if let Some(t) = app.selected_target() {
                app.pull_hardware(t);
            }
        }
        KeyCode::Char('s') => {
            if let Some(t) = app.selected_target() {
                app.open_system(Some(t));
            }
        }
        KeyCode::Char('K') => {
            let host = app.selected_target().map(|t| t.ssh_target.split('@').next_back().unwrap_or("").to_string()).filter(|h| !h.is_empty());
            let mut lines = vec!["Makes sure root on this machine has an SSH key (the nix daemon connects to builders as root) and prints its public key.".to_string()];
            if let Some(h) = &host {
                lines.push(format!("Also records the host key of {h} for root. Then authorize the printed key on {h} to use it as a builder."));
            }
            let root = deploy::running_as_root();
            app.prompt = Some(Prompt { title: "Root SSH key for remote builders".into(), lines, action: PromptAction::RootKey(host), password: String::new(), ask_password: !root, require_password: !root });
        }
        _ => {}
    }
    true
}

fn key_target_edit(app: &mut App, key: KeyEvent) -> bool {
    let cur = TFIELDS[app.tform.cursor.min(TFIELDS.len() - 1)];
    match key.code {
        KeyCode::Esc => app.screen = Screen::Targets,
        KeyCode::Up => app.tform.cursor = app.tform.cursor.saturating_sub(1),
        KeyCode::Down | KeyCode::Tab => app.tform.cursor = (app.tform.cursor + 1).min(TFIELDS.len() - 1),
        KeyCode::Enter if cur == TField::Save => app.save_target_form(),
        KeyCode::Enter => app.tform.cursor = (app.tform.cursor + 1).min(TFIELDS.len() - 1),
        KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right if cur == TField::Sudo => app.tform.sudo = !app.tform.sudo,
        KeyCode::Backspace => {
            if let Some(s) = app.tform.text_mut(cur) {
                s.pop();
            }
        }
        KeyCode::Char(c) => {
            if let Some(s) = app.tform.text_mut(cur) {
                if cur != TField::Port || c.is_ascii_digit() {
                    s.push(c);
                }
            }
        }
        _ => {}
    }
    true
}

fn key_job(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.screen = app.job_back,
        KeyCode::Char('s') => {
            if let Some(id) = app.job_id.clone() {
                let jobs = app.jobs.clone();
                let tx = app.tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = jobs.stop(&id).await {
                        let _ = tx.send(Msg::Error(e.to_string()));
                    }
                });
            }
        }
        KeyCode::Up => {
            app.job_follow = false;
            app.job_scroll = app.job_scroll.saturating_sub(1);
        }
        KeyCode::Down => app.job_scroll += 1,
        KeyCode::PageUp => {
            app.job_follow = false;
            app.job_scroll = app.job_scroll.saturating_sub(20);
        }
        KeyCode::PageDown => app.job_scroll += 20,
        KeyCode::End | KeyCode::Char('f') => app.job_follow = true,
        KeyCode::Char('j') => app.screen = Screen::Jobs,
        _ => {}
    }
    true
}

fn key_jobs(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.screen = app.job_back,
        KeyCode::Up => {
            let i = app.jobs_list.selected().unwrap_or(0);
            app.jobs_list.select(Some(i.saturating_sub(1)));
        }
        KeyCode::Down => {
            let i = app.jobs_list.selected().unwrap_or(0);
            if !app.job_ids.is_empty() {
                app.jobs_list.select(Some((i + 1).min(app.job_ids.len() - 1)));
            }
        }
        KeyCode::Enter => {
            if let Some(id) = app.jobs_list.selected().and_then(|i| app.job_ids.get(i)).cloned() {
                app.open_job(id);
            }
        }
        _ => {}
    }
    true
}

// ───────────────────────── drawing ─────────────────────────

type Titles = HashMap<String, (String, JobStatus)>;

fn draw(f: &mut Frame, app: &mut App, titles: &Titles) {
    let area = f.area();
    let chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(3), Constraint::Length(1)]).split(area);
    match app.screen {
        Screen::Pick => draw_pick(f, app, chunks[0]),
        Screen::Workspace => draw_workspace(f, app, chunks[0]),
        Screen::Build => draw_build(f, app, chunks[0]),
        Screen::System => draw_system(f, app, chunks[0]),
        Screen::Targets => draw_targets(f, app, chunks[0]),
        Screen::TargetEdit => draw_target_edit(f, app, chunks[0]),
        Screen::Job => draw_job(f, app, chunks[0]),
        Screen::Jobs => draw_jobs(f, app, chunks[0], titles),
    }
    let help = if app.prompt.is_some() {
        " Enter confirm · Esc cancel "
    } else {
        match app.screen {
            Screen::Pick => " ↑↓ choose · Enter open · / or Tab type a path · t targets · q quit ",
            Screen::Workspace => " b build · v validate · g system · t targets · j jobs · m free memory · h next host · r rescan · q back ",
            Screen::Build => " ↑↓ field · ←→/Space change · type passwords · Enter on Start · Esc back ",
            Screen::System => " ↑↓ choose · Tab switch list · Enter roll back / delete · c collect garbage · o older than 30d · D all old · r reload · Esc back ",
            Screen::Targets => " a add · e edit · d remove · c check · t test ssh · k key access · p pull hardware · s system · K root key · Esc back ",
            Screen::TargetEdit => " ↑↓/Tab field · type · Space toggle · Enter on Save · Esc cancel ",
            Screen::Job => " s stop · ↑↓ PgUp PgDn scroll · f follow · j jobs · Esc back ",
            Screen::Jobs => " ↑↓ choose · Enter open · Esc back ",
        }
    };
    let status = app.status.clone().unwrap_or_default();
    let line = Line::from(vec![Span::styled(help, Style::default().fg(Color::Black).bg(Color::Cyan)), Span::raw(" "), Span::styled(status, Style::default().fg(Color::Yellow))]);
    f.render_widget(Paragraph::new(line), chunks[1]);
    if let Some(p) = &app.prompt {
        draw_prompt(f, p, chunks[0]);
    }
}

fn block(title: &str) -> Block<'_> {
    Block::default().borders(Borders::ALL).title(Span::styled(format!(" {title} "), Style::default().add_modifier(Modifier::BOLD)))
}

fn key_span(k: &str) -> Span<'static> {
    Span::styled(k.to_string(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
}

fn draw_prompt(f: &mut Frame, p: &Prompt, area: Rect) {
    let w = area.width.min(80);
    let inner = w.saturating_sub(2).max(1) as usize;
    let wrapped: usize = p.lines.iter().map(|l| l.chars().count().max(1).div_ceil(inner)).sum();
    let h = ((wrapped as u16) + 2 + if p.ask_password { 2 } else { 1 }).min(area.height);
    let popup = Rect { x: area.x + (area.width.saturating_sub(w)) / 2, y: area.y + (area.height.saturating_sub(h)) / 2, width: w, height: h };
    f.render_widget(Clear, popup);
    let mut text: Vec<Line> = p.lines.iter().map(|l| Line::from(Span::styled(l.clone(), Style::default().fg(Color::Yellow)))).collect();
    text.push(Line::raw(""));
    if p.ask_password {
        text.push(Line::from(vec![Span::raw("sudo password: "), Span::styled("•".repeat(p.password.len()), Style::default().fg(Color::Cyan)), Span::raw("▏")]));
    }
    text.push(Line::from(Span::styled("Enter confirm · Esc cancel", Style::default().fg(Color::DarkGray))));
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }).block(block(&p.title)), popup);
}

fn draw_pick(f: &mut Frame, app: &mut App, area: Rect) {
    let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(5)]).split(area);
    f.render_widget(Paragraph::new("Nixos-manager — pick the folder that holds your NixOS configuration (flake.nix or configuration.nix)").style(Style::default().add_modifier(Modifier::BOLD)).block(Block::default().borders(Borders::ALL)), rows[0]);
    let input_style = if app.pick_input_focus { Style::default().fg(Color::Cyan) } else { Style::default() };
    f.render_widget(Paragraph::new(format!("{}{}", app.pick_input, if app.pick_input_focus { "▏" } else { "" })).style(input_style).block(block("Path (e.g. ~/nixos or /etc/nixos)")), rows[1]);
    let entries = app.pick_entries();
    let items: Vec<ListItem> = entries.iter().map(|(p, d)| ListItem::new(Line::from(vec![Span::raw(p.clone()), Span::styled(format!("  {d}"), Style::default().fg(Color::DarkGray))]))).collect();
    let list = List::new(items).block(block("Recent and found on this machine")).highlight_style(Style::default().bg(Color::Blue).fg(Color::White)).highlight_symbol("▶ ");
    f.render_stateful_widget(list, rows[2], &mut app.pick_list);
}

fn draw_workspace(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(40), Constraint::Percentage(60)]).split(area);
    let Some(ov) = &app.overview else {
        f.render_widget(Paragraph::new("Loading…").block(block("Workspace")), area);
        return;
    };
    let files: Vec<ListItem> = ov.files.iter().filter(|x| x.rel_path.ends_with(".nix")).map(|x| {
        let mut spans = vec![Span::raw(x.rel_path.clone())];
        if !x.parse_ok {
            spans.push(Span::styled("  syntax error", Style::default().fg(Color::Red)));
        } else if ov.unreachable.contains(&x.rel_path) {
            spans.push(Span::styled("  not imported", Style::default().fg(Color::DarkGray)));
        }
        ListItem::new(Line::from(spans))
    }).collect();
    f.render_widget(List::new(files).block(block("Nix files")), cols[0]);
    let dim = Style::default().fg(Color::DarkGray);
    let mut lines: Vec<Line> = vec![
        Line::from(vec![Span::styled("Folder   ", dim), Span::raw(ov.location.path.clone())]),
        Line::from(vec![Span::styled("Type     ", dim), Span::raw(format!("{:?}", ov.location.kind).to_lowercase())]),
        Line::from(vec![Span::styled("Host     ", dim), Span::styled(app.host.clone().unwrap_or_else(|| "-".into()), Style::default().fg(Color::Cyan)), Span::styled(if ov.location.hosts.len() > 1 { format!("  (h cycles: {})", ov.location.hosts.join(", ")) } else { String::new() }, dim)]),
        Line::from(vec![Span::styled("Machine  ", dim), Span::raw(format!("{} · NixOS {}", ov.machine.hostname.clone().unwrap_or_default(), ov.machine.nixos_version.clone().unwrap_or_default()))]),
        Line::from(vec![Span::styled("Writable ", dim), Span::raw(if ov.location.writable { "yes" } else { "no" })]),
    ];
    if deploy::running_as_root() {
        lines.push(Line::from(vec![Span::styled("Running  ", dim), Span::styled("as root — no sudo password is asked for local actions", Style::default().fg(Color::Green))]));
    }
    if let Some(hw) = &app.hw {
        let txt = match (&hw.pulled_from, hw.local_exists, hw.system_exists, hw.differs) {
            (Some(p), _, _, _) => format!("{} is the copy pulled from {} ({})", hw.rel_path, p.target, p.at.get(..16).unwrap_or(&p.at).replace('T', " ")),
            (None, false, _, _) => format!("no {} in the working copy", hw.rel_path),
            (None, true, false, _) => format!("{} present; this machine has none in /etc/nixos", hw.rel_path),
            (None, true, true, true) => format!("{} differs from /etc/nixos (local builds use the system copy)", hw.rel_path),
            (None, true, true, false) => format!("{} matches /etc/nixos", hw.rel_path),
        };
        lines.push(Line::from(vec![Span::styled("Hardware ", dim), Span::raw(txt)]));
    }
    let broken = ov.files.iter().filter(|x| !x.parse_ok).count();
    if broken > 0 {
        lines.push(Line::from(Span::styled(format!("{broken} file(s) have syntax errors"), Style::default().fg(Color::Red))));
    }
    if let Some(bs) = &app.build_status {
        match (&bs.last_build_at, bs.changed_files.len()) {
            (Some(at), 0) => lines.push(Line::from(Span::styled(format!("Last successful build {} · nothing changed since", at.get(..16).unwrap_or(at).replace('T', " ")), Style::default().fg(Color::Green)))),
            (Some(at), n) => lines.push(Line::from(Span::styled(format!("{n} file(s) changed since the last build ({}) — press b to build", at.get(..16).unwrap_or(at).replace('T', " ")), Style::default().fg(Color::Yellow)))),
            (None, _) => lines.push(Line::from(Span::styled("No build recorded yet for this folder", dim))),
        }
    }
    lines.push(Line::raw(""));
    if app.targets.is_empty() {
        lines.push(Line::from(vec![Span::styled("No deploy targets yet — press ", dim), key_span("t"), Span::styled(" to add one; builds apply to this machine.", dim)]));
    } else {
        lines.push(Line::from(Span::styled("Deploy targets (t manages them):", dim)));
        for t in &app.targets {
            let mut spans = vec![Span::raw(format!("  {}  {}{}", t.name, t.ssh_target, t.port.map(|p| format!(":{p}")).unwrap_or_default()))];
            if let Some(c) = app.checks.get(&t.name) {
                spans.push(if c.reachable { Span::styled("  reachable", Style::default().fg(Color::Green)) } else { Span::styled("  unreachable", Style::default().fg(Color::Red)) });
            }
            lines.push(Line::from(spans));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![key_span("b"), Span::raw(" Build & apply    "), key_span("v"), Span::raw(" Validate (no sudo)    "), key_span("g"), Span::raw(" System: generations, disk, garbage    "), key_span("j"), Span::raw(" Jobs")]));
    lines.push(Line::from(vec![key_span("t"), Span::raw(" Targets: add, check, key access, pull hardware, remote system    "), key_span("m"), Span::raw(" Free memory (log out other sessions, drop caches)")]));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block("Configuration")), cols[1]);
}

fn draw_build(f: &mut Frame, app: &mut App, area: Rect) {
    let hosts: Vec<String> = app.overview.as_ref().map(|o| o.location.hosts.clone()).unwrap_or_default();
    let fields = app.form.fields(!app.targets.is_empty(), hosts.len());
    let cursor = app.form.cursor.min(fields.len() - 1);
    let target = app.form_target();
    let builder = app.form_builder();
    let mut lines: Vec<Line> = Vec::new();
    let action_label = match app.form.action {
        RebuildAction::Switch => "switch — build, activate now, and make default at boot",
        RebuildAction::Boot => "boot — build and make default at next boot",
        RebuildAction::Test => "test — build and activate until the next boot",
        RebuildAction::Build => "build — only build",
        RebuildAction::DryBuild => "dry-build — only check that it builds",
    };
    for (i, fld) in fields.iter().enumerate() {
        let sel = i == cursor;
        let (label, value) = match fld {
            Field::DeployTo => ("Deploy to", match &target { Some(t) => format!("{} ({})", t.name, t.ssh_target), None => "This machine (sync to /etc/nixos, then rebuild)".into() }),
            Field::Mode => ("Mode", if app.form.on_target { "Build and activate on the target (copy the working copy there)".into() } else { "Build here, send the result to the target".into() }),
            Field::BuildOn => ("Build on", match &builder { Some(t) => format!("{} ({}) — evaluated here, compiled there, activated here", t.name, t.ssh_target), None => "this machine".into() }),
            Field::Host => ("Host", hosts.get(app.form.host_idx).cloned().unwrap_or_default()),
            Field::Action => ("Action", action_label.into()),
            Field::Upgrade => ("Upgrade", if app.form.upgrade { "yes — update all flake inputs".into() } else { "no — keep the pinned inputs".into() }),
            Field::SkipSync => ("Skip sync", if app.form.skip_sync { "yes — rebuild /etc/nixos as it is".into() } else { "no — copy the working copy to /etc/nixos first".into() }),
            Field::Hardware => ("Hardware", if app.form.workspace_hw { "working copy's hardware-configuration.nix (the target's own file is ignored)".into() } else { "the target's own /etc/nixos/hardware-configuration.nix".into() }),
            Field::After => (if target.is_some() { "Target afterwards" } else { "Afterwards" }, after_label(app.form.after).into()),
            Field::AfterLocal => ("This machine afterwards", after_label(app.form.after_local).into()),
            Field::AfterAlways => ("Even on failure", if app.form.after_always { "yes — reboot / power off even if the build fails".into() } else { "no — only after a successful build".into() }),
            Field::LocalPassword => ("Your sudo password", "•".repeat(app.form.local_password.len())),
            Field::TargetPassword => (if target.is_some() { "sudo password on the target" } else { "sudo password on the builder" }, "•".repeat(app.form.target_password.len())),
            Field::Start => ("", "Start".into()),
        };
        let style = if sel { Style::default().bg(Color::Blue).fg(Color::White) } else { Style::default() };
        let label_s = if label.is_empty() { String::new() } else { format!("{label:<28}") };
        lines.push(Line::from(vec![Span::styled(if sel { "▶ " } else { "  " }, style), Span::styled(label_s, style.fg(if sel { Color::White } else { Color::DarkGray })), Span::styled(value, style)]));
    }
    lines.push(Line::raw(""));
    let dim = Style::default().fg(Color::DarkGray);
    if let Some(t) = target.as_ref().or(builder.as_ref()) {
        let role = if target.is_some() { "target" } else { "builder" };
        if app.checking.contains(&t.name) {
            lines.push(Line::from(Span::styled(format!("Checking the {role}…"), Style::default().fg(Color::Yellow))));
        } else if let Some(c) = app.checks.get(&t.name) {
            if !c.reachable {
                lines.push(Line::from(Span::styled(format!("Cannot reach {}: {}", t.ssh_target, c.message.clone().unwrap_or_default()), Style::default().fg(Color::Red))));
            } else {
                lines.push(Line::raw(format!(
                    "{} {} · NixOS {} · trusted by nix: {} · passwordless sudo: {}",
                    role,
                    c.hostname.clone().unwrap_or_default(),
                    c.nixos_version.clone().unwrap_or_default(),
                    if c.is_root { "root".to_string() } else { yes_no(c.trusted).to_string() },
                    if c.is_root { "n/a".to_string() } else { yes_no(c.passwordless_sudo).to_string() }
                )));
                let need = if target.is_some() && app.form.on_target { !c.is_root && c.passwordless_sudo != Some(true) } else if target.is_some() { c.needs_sudo_password } else { !c.is_root && c.passwordless_sudo != Some(true) };
                if need {
                    lines.push(Line::from(Span::styled(format!("The {role}'s sudo password is needed (type it in the field above)."), Style::default().fg(Color::Yellow))));
                }
                if target.is_some() && !app.form.on_target {
                    match c.hardware_config.as_deref() {
                        Some("none") if !app.form.workspace_hw => lines.push(Line::from(Span::styled("The target has no /etc/nixos/hardware-configuration.nix — switch Hardware to the working copy's file.", Style::default().fg(Color::Red)))),
                        Some("generate") if !app.form.workspace_hw => lines.push(Line::from(Span::styled("The target has no hardware-configuration.nix in /etc/nixos; nixos-generate-config runs there to produce one.", Style::default().fg(Color::Yellow)))),
                        _ => {}
                    }
                }
                if target.is_some() && app.form.workspace_hw {
                    lines.push(Line::from(Span::styled("Warning: the working copy's hardware-configuration.nix is used. If it was written for another machine the target may not boot.", Style::default().fg(Color::Yellow))));
                }
            }
        }
    }
    if let Some(t) = &target {
        if !app.form.on_target {
            lines.push(Line::from(Span::styled(format!("Build here, send the result to {}: its flake.lock is kept; temporary nix trust is granted when needed.", t.ssh_target), dim)));
        } else {
            lines.push(Line::from(Span::styled("On the target: the working copy is copied there, synced to its /etc/nixos and rebuilt as root (flakes and unfree allowed through the environment).", dim)));
        }
    } else if let Some(b) = &builder {
        lines.push(Line::from(Span::styled(format!("Same steps as sync.sh, as root, with nixos-rebuild --build-host {}: the system is evaluated here, compiled on the builder, copied back and activated here.", b.ssh_target), dim)));
    } else {
        lines.push(Line::from(Span::styled("Same steps as sync.sh, as root: preserve /etc/nixos/flake.lock, copy hardware-configuration.nix from /etc/nixos, clear /etc/nixos, copy the working copy, nixos-rebuild.", dim)));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block("Build & apply")), area);
}

fn draw_system(f: &mut Frame, app: &mut App, area: Rect) {
    let others_h = if app.sys.others_loading || app.sys.others_error.is_some() { 3 } else { (app.sys.others.len() as u16 + 2).clamp(3, 10) };
    let rows = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(4), Constraint::Min(4), Constraint::Length(others_h)]).split(area);
    let machine = match &app.sys.machine {
        None => "This machine".to_string(),
        Some(t) => format!("{} ({}{})", t.name, t.ssh_target, t.port.map(|p| format!(":{p}")).unwrap_or_default()),
    };
    let dim = Style::default().fg(Color::DarkGray);
    let mut head: Vec<Line> = vec![Line::from(vec![Span::styled("Machine  ", dim), Span::styled(machine, Style::default().fg(Color::Cyan)), Span::raw(app.sys.hostname.as_ref().map(|h| format!("  host {h}")).unwrap_or_default())])];
    match (&app.sys.store, app.sys.loading) {
        (Some(s), _) => head.push(Line::from(vec![Span::styled("Disk     ", dim), Span::raw(format!("/nix/store on a {} filesystem · {} used · {} free · {} generations", human(s.size), human(s.used), human(s.avail), s.generations))])),
        (None, true) => head.push(Line::from(Span::styled("Loading…", Style::default().fg(Color::Yellow)))),
        (None, false) => head.push(Line::from(vec![Span::styled("Disk     ", dim), Span::raw("unknown")])),
    }
    if let Some(t) = &app.sys.machine {
        let access = match app.remote_needs_password(t) {
            Some(false) => "root actions need no password".to_string(),
            Some(true) => "root actions ask for the target's sudo password".to_string(),
            None if app.checking.contains(&t.name) => "checking access…".to_string(),
            None => "access not checked".to_string(),
        };
        head.push(Line::from(vec![Span::styled("Access   ", dim), Span::raw(access)]));
    }
    f.render_widget(Paragraph::new(head).block(block("System")), rows[0]);

    let gens: Vec<ListItem> = if let Some(e) = &app.sys.error {
        vec![ListItem::new(Line::from(Span::styled(e.clone(), Style::default().fg(Color::Red))))]
    } else if app.sys.loading {
        vec![ListItem::new("Loading generations…")]
    } else {
        app.sys.generations.iter().map(|g| {
            let mut spans = vec![Span::styled(format!("#{:<5}", g.number), Style::default().fg(Color::Cyan)), Span::raw(format!("{}  NixOS {}", g.date, g.nixos_version.clone().unwrap_or_default()))];
            if let Some(k) = &g.kernel {
                spans.push(Span::styled(format!("  kernel {k}"), dim));
            }
            if g.current {
                spans.push(Span::styled("  current", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)));
            }
            ListItem::new(Line::from(spans))
        }).collect()
    };
    let gen_style = if app.sys.focus_other { Style::default().bg(Color::DarkGray).fg(Color::White) } else { Style::default().bg(Color::Blue).fg(Color::White) };
    let list = List::new(gens).block(block("Generations — Enter rolls back to the selected one · c / o / D collect garbage")).highlight_style(gen_style).highlight_symbol("▶ ");
    f.render_stateful_widget(list, rows[1], &mut app.sys.gen_list);

    let others: Vec<ListItem> = if let Some(e) = &app.sys.others_error {
        vec![ListItem::new(Line::from(Span::styled(e.clone(), Style::default().fg(Color::Red))))]
    } else if app.sys.others_loading {
        vec![ListItem::new("Scanning the store for systems built for other machines…")]
    } else if app.sys.others.is_empty() {
        vec![ListItem::new(Line::from(Span::styled("None: every system in the store belongs to a generation of this machine.", dim)))]
    } else {
        app.sys.others.iter().map(|o| {
            let mut spans = vec![Span::styled(format!("{:<18}", o.host), Style::default().fg(Color::Cyan)), Span::raw(format!("NixOS {}  only this build {}  whole closure {}", o.nixos_version, human(o.exclusive_size), human(o.closure_size)))];
            if !o.roots.is_empty() {
                spans.push(Span::styled(format!("  still referenced by {}", o.roots.join(", ")), Style::default().fg(Color::Yellow)));
            }
            ListItem::new(Line::from(spans))
        }).collect()
    };
    let other_style = if app.sys.focus_other { Style::default().bg(Color::Blue).fg(Color::White) } else { Style::default().bg(Color::DarkGray).fg(Color::White) };
    let list = List::new(others).block(block("Builds for other machines — Tab here, Enter deletes the selected one and collects garbage")).highlight_style(other_style).highlight_symbol("▶ ");
    f.render_stateful_widget(list, rows[2], &mut app.sys.other_list);
}

fn draw_targets(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(45), Constraint::Percentage(55)]).split(area);
    let dim = Style::default().fg(Color::DarkGray);
    let items: Vec<ListItem> = if app.targets.is_empty() {
        vec![ListItem::new(Line::from(Span::styled("No targets yet — press a to add one.", dim)))]
    } else {
        app.targets.iter().map(|t| {
            let mut spans = vec![Span::styled(format!("{:<16}", t.name), Style::default().fg(Color::Cyan)), Span::raw(format!("{}{}", t.ssh_target, t.port.map(|p| format!(":{p}")).unwrap_or_default()))];
            if app.checking.contains(&t.name) {
                spans.push(Span::styled("  checking…", Style::default().fg(Color::Yellow)));
            } else if let Some(c) = app.checks.get(&t.name) {
                spans.push(if c.reachable { Span::styled("  reachable", Style::default().fg(Color::Green)) } else { Span::styled("  unreachable", Style::default().fg(Color::Red)) });
            }
            ListItem::new(Line::from(spans))
        }).collect()
    };
    let list = List::new(items).block(block("Deploy targets")).highlight_style(Style::default().bg(Color::Blue).fg(Color::White)).highlight_symbol("▶ ");
    f.render_stateful_widget(list, cols[0], &mut app.target_list);

    let mut lines: Vec<Line> = Vec::new();
    if let Some(t) = app.selected_target() {
        lines.push(Line::from(vec![Span::styled("Name        ", dim), Span::raw(t.name.clone())]));
        lines.push(Line::from(vec![Span::styled("SSH         ", dim), Span::raw(format!("{}{}", t.ssh_target, t.port.map(|p| format!("  port {p}")).unwrap_or_default()))]));
        lines.push(Line::from(vec![Span::styled("Flake host  ", dim), Span::raw(t.flake_host.clone().unwrap_or_else(|| "(the host selected in the workspace)".into()))]));
        let is_root = t.ssh_target.starts_with("root@");
        lines.push(Line::from(vec![Span::styled("Activation  ", dim), match (t.use_remote_sudo, is_root) {
            (true, _) => Span::raw("with sudo on the target"),
            (false, true) => Span::raw("directly (the SSH user is root)"),
            (false, false) => Span::styled("without sudo — fails unless the SSH user is root (edit with e)", Style::default().fg(Color::Yellow)),
        }]));
        lines.push(Line::raw(""));
        if app.checking.contains(&t.name) {
            lines.push(Line::from(Span::styled("Checking…", Style::default().fg(Color::Yellow))));
        } else if let Some(c) = app.checks.get(&t.name) {
            if !c.reachable {
                lines.push(Line::from(Span::styled(format!("Unreachable: {}", c.message.clone().unwrap_or_default()), Style::default().fg(Color::Red))));
                lines.push(Line::from(Span::styled("Press t to run ssh verbosely, or k to install this app's key with the target's password.", dim)));
            } else {
                lines.push(Line::from(Span::styled("Reachable", Style::default().fg(Color::Green))));
                lines.push(Line::from(vec![Span::styled("Machine     ", dim), Span::raw(format!("{} · NixOS {}", c.hostname.clone().unwrap_or_default(), c.nixos_version.clone().unwrap_or_default()))]));
                lines.push(Line::from(vec![Span::styled("SSH user    ", dim), Span::raw(format!("{}{}", c.user, if c.is_root { " (root)" } else { "" }))]));
                if !c.is_root {
                    lines.push(Line::from(vec![Span::styled("nix trust   ", dim), Span::raw(yes_no(c.trusted)), Span::styled("  (granted temporarily during a deploy when missing)", dim)]));
                    lines.push(Line::from(vec![Span::styled("sudo        ", dim), Span::raw(if c.passwordless_sudo == Some(true) { "passwordless" } else { "asks for a password (root actions prompt for it)" })]));
                }
                lines.push(Line::from(vec![Span::styled("Hardware    ", dim), Span::raw(match c.hardware_config.as_deref() { Some("etc") => "/etc/nixos/hardware-configuration.nix present", Some("generate") => "none in /etc/nixos; nixos-generate-config would produce one", Some("none") => "none — deploy with the working copy's file", _ => "unknown" })]));
                if let Some(m) = &c.message {
                    lines.push(Line::from(Span::styled(m.clone(), Style::default().fg(Color::Yellow))));
                }
            }
        } else {
            lines.push(Line::from(Span::styled("Not checked yet — press c (or Enter).", dim)));
        }
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(vec![key_span("a"), Span::raw(" add    "), key_span("e"), Span::raw(" edit    "), key_span("d"), Span::raw(" remove    "), key_span("c"), Span::raw(" check reachability, trust, sudo, hardware")]));
    lines.push(Line::from(vec![key_span("t"), Span::raw(" test the SSH connection (job log)    "), key_span("k"), Span::raw(" install this app's SSH key using the target's password once")]));
    lines.push(Line::from(vec![key_span("p"), Span::raw(" pull the target's hardware-configuration.nix into the working copy    "), key_span("s"), Span::raw(" system: generations, roll back, disk, garbage")]));
    lines.push(Line::from(vec![key_span("K"), Span::raw(" root SSH key on this machine, to use the target as a builder (Build on, in the build form)")]));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block("Target")), cols[1]);
}

fn draw_target_edit(f: &mut Frame, app: &mut App, area: Rect) {
    let cursor = app.tform.cursor.min(TFIELDS.len() - 1);
    let mut lines: Vec<Line> = Vec::new();
    for (i, fld) in TFIELDS.iter().enumerate() {
        let sel = i == cursor;
        let caret = if sel { "▏" } else { "" };
        let (label, value) = match fld {
            TField::Name => ("Name", format!("{}{caret}", app.tform.name)),
            TField::Host => ("Host (name or IP)", format!("{}{caret}", app.tform.host)),
            TField::User => ("SSH user", format!("{}{caret}", if app.tform.user.is_empty() && !sel { "(your user name)".to_string() } else { app.tform.user.clone() })),
            TField::Port => ("Port", format!("{}{caret}", if app.tform.port.is_empty() && !sel { "22".to_string() } else { app.tform.port.clone() })),
            TField::FlakeHost => ("nixosConfigurations.<name>", format!("{}{caret}", if app.tform.flake_host.is_empty() && !sel { "(the host selected in the workspace)".to_string() } else { app.tform.flake_host.clone() })),
            TField::Sudo => ("Use sudo for activation", if app.tform.user.trim() == "root" { "not needed — the SSH user is root".into() } else if app.tform.sudo { "yes (needed unless the SSH user is root)".into() } else { "no".into() }),
            TField::Save => ("", "Save".into()),
        };
        let style = if sel { Style::default().bg(Color::Blue).fg(Color::White) } else { Style::default() };
        let label_s = if label.is_empty() { String::new() } else { format!("{label:<30}") };
        lines.push(Line::from(vec![Span::styled(if sel { "▶ " } else { "  " }, style), Span::styled(label_s, style.fg(if sel { Color::White } else { Color::DarkGray })), Span::styled(value, style)]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled("The address is stored as user@host; the port only when it is not 22. Check the target afterwards with c, and install the key with k if ssh still asks for a password.", Style::default().fg(Color::DarkGray))));
    let title = if app.tform.edit_idx.is_some() { "Edit target" } else { "Add target" };
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block(title)), area);
}

fn draw_job(f: &mut Frame, app: &mut App, area: Rect) {
    let Some(d) = &app.job else {
        f.render_widget(Paragraph::new("Loading job…").block(block("Job")), area);
        return;
    };
    let status_style = match d.info.status {
        JobStatus::Succeeded => Style::default().fg(Color::Green),
        JobStatus::Failed => Style::default().fg(Color::Red),
        JobStatus::Cancelled => Style::default().fg(Color::Yellow),
        _ => Style::default().fg(Color::Cyan),
    };
    let title = format!("{} — {:?}{}", d.info.title, d.info.status, d.info.error.as_ref().map(|e| format!(": {e}")).unwrap_or_default());
    let inner_h = area.height.saturating_sub(2) as usize;
    let total = d.log.len();
    if app.job_follow {
        app.job_scroll = total.saturating_sub(inner_h);
    }
    app.job_scroll = app.job_scroll.min(total.saturating_sub(inner_h));
    let lines: Vec<Line> = d.log.iter().skip(app.job_scroll).take(inner_h).map(|l| {
        let st = match l.stream {
            LogStream::System => Style::default().fg(Color::Cyan),
            LogStream::Stderr => Style::default().fg(Color::Yellow),
            LogStream::Stdout => Style::default(),
        };
        let st = if l.text.starts_with("error:") || l.text.contains("] ERROR") { Style::default().fg(Color::Red) } else { st };
        Line::from(Span::styled(l.text.clone(), st))
    }).collect();
    let b = Block::default().borders(Borders::ALL).title(Span::styled(format!(" {title} "), status_style.add_modifier(Modifier::BOLD))).title_bottom(Line::from(Span::styled(format!(" {}/{} lines{} ", (app.job_scroll + lines.len()).min(total), total, if app.job_follow { " · following" } else { "" }), Style::default().fg(Color::DarkGray))));
    f.render_widget(Paragraph::new(lines).block(b), area);
}

fn draw_jobs(f: &mut Frame, app: &mut App, area: Rect, titles: &Titles) {
    let items: Vec<ListItem> = app.job_ids.iter().map(|id| {
        let (t, s) = titles.get(id).cloned().unwrap_or((id.clone(), JobStatus::Pending));
        let st = match s { JobStatus::Succeeded => Color::Green, JobStatus::Failed => Color::Red, JobStatus::Cancelled => Color::Yellow, _ => Color::Cyan };
        ListItem::new(Line::from(vec![Span::styled(format!("{:<10}", format!("{s:?}").to_lowercase()), Style::default().fg(st)), Span::raw(t)]))
    }).collect();
    let list = List::new(items).block(block("Jobs of this session (older logs: ~/.local/state/nixos-manager/logs)")).highlight_style(Style::default().bg(Color::Blue).fg(Color::White)).highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.jobs_list);
}

// ───────────────────────── main loop ─────────────────────────

fn debug_log(msg: &Msg) {
    let Ok(f) = std::env::var("NM_TUI_LOG") else { return };
    use std::io::Write;
    let what = match msg {
        Msg::State(_) => "state".to_string(),
        Msg::Suggestions(s) => format!("suggestions {}", s.len()),
        Msg::Opened(r) => format!("opened {:?}", r.as_ref().map(|x| x.0.display().to_string())),
        Msg::Check(n, c) => format!("check {n} reachable={}", c.reachable),
        Msg::SystemInfo(r) => format!("system {:?}", r.as_ref().map(|i| i.generations.len())),
        Msg::OtherSystems(m, r) => format!("others {m:?} {:?}", r.as_ref().map(|o| o.len())),
        Msg::Job(_) => "job".into(),
        Msg::OpenJob(id) => format!("open-job {id}"),
        Msg::JobDetail(d) => format!("job-detail {} lines", d.log.len()),
        Msg::Info(s) => format!("info {s}"),
        Msg::Error(e) => format!("error {e}"),
    };
    if let Ok(mut fh) = std::fs::OpenOptions::new().create(true).append(true).open(f) {
        let _ = writeln!(fh, "{what}");
    }
}

fn apply_msg(app: &mut App, msg: Msg, titles: &mut Titles) {
    match msg {
        Msg::State(st) => {
            app.targets = st.deploy_targets.clone();
            app.state = Some(st);
            if let Some(i) = app.target_list.selected() {
                if i >= app.targets.len() {
                    app.target_list.select(Some(app.targets.len().saturating_sub(1)));
                }
            }
            if app.form.target_idx > app.targets.len() {
                app.form.target_idx = 0;
            }
            if app.form.build_on_idx > app.targets.len() {
                app.form.build_on_idx = 0;
            }
        }
        Msg::Suggestions(s) => app.suggestions = s,
        Msg::Opened(Ok((root, ov, bs, hw))) => {
            app.host = ov.location.hosts.iter().find(|h| Some(*h) == ov.machine.hostname.as_ref()).or(ov.location.hosts.first()).cloned();
            if let Some(st) = &app.state {
                if let Some(h) = st.recent_locations.iter().find(|r| r.path == root.to_string_lossy()).and_then(|r| r.host.clone()) {
                    if ov.location.hosts.contains(&h) {
                        app.host = Some(h);
                    }
                }
            }
            app.root = Some(root);
            app.overview = Some(ov);
            app.build_status = bs;
            app.hw = Some(hw);
            app.status = None;
            if matches!(app.screen, Screen::Pick | Screen::Workspace) {
                app.screen = Screen::Workspace;
            }
        }
        Msg::Opened(Err(e)) => app.status = Some(e),
        Msg::Check(name, c) => {
            app.checking.remove(&name);
            app.checks.insert(name, c);
        }
        Msg::SystemInfo(Ok(info)) => {
            if info.machine == app.sys.machine_name() {
                app.sys.loading = false;
                app.sys.hostname = info.hostname;
                app.sys.generations = info.generations;
                app.sys.store = info.store;
                app.sys.gen_list.select(Some(0));
            }
        }
        Msg::SystemInfo(Err(e)) => {
            app.sys.loading = false;
            app.sys.error = Some(e);
        }
        Msg::OtherSystems(machine, res) => {
            if machine == app.sys.machine_name() {
                app.sys.others_loading = false;
                match res {
                    Ok(o) => {
                        app.sys.others = o;
                        app.sys.other_list.select(Some(0));
                    }
                    Err(e) => app.sys.others_error = Some(e),
                }
            }
        }
        Msg::Job(ev) => {
            match &ev {
                JobEvent::Created { job } => {
                    titles.insert(job.id.clone(), (job.title.clone(), job.status));
                    if !app.job_ids.contains(&job.id) {
                        app.job_ids.insert(0, job.id.clone());
                    }
                }
                JobEvent::Status { job_id, status, .. } => {
                    if let Some(t) = titles.get_mut(job_id) {
                        t.1 = *status;
                    }
                    // a finished system job changes what the system screen shows
                    if app.screen == Screen::System && matches!(status, JobStatus::Succeeded | JobStatus::Failed) {
                        if let Some(t) = titles.get(job_id) {
                            if t.0.starts_with("Roll back") || t.0.starts_with("Collect garbage") || t.0.starts_with("Delete ") {
                                app.load_system();
                            }
                        }
                    }
                }
                _ => {}
            }
            let mine = match &ev {
                JobEvent::Log { job_id, .. } | JobEvent::Status { job_id, .. } => Some(job_id.clone()),
                JobEvent::Created { job } => Some(job.id.clone()),
            };
            if mine.as_deref() == app.job_id.as_deref() {
                match ev {
                    JobEvent::Log { line, .. } => {
                        if let Some(d) = app.job.as_mut() {
                            d.log.push(line);
                            d.info.log_lines = d.log.len() as u32;
                        } else {
                            app.refresh_job();
                        }
                    }
                    JobEvent::Status { status, error, .. } => {
                        if let Some(d) = app.job.as_mut() {
                            d.info.status = status;
                            d.info.error = error;
                        } else {
                            app.refresh_job();
                        }
                    }
                    JobEvent::Created { .. } => app.refresh_job(),
                }
            }
        }
        Msg::OpenJob(id) => app.open_job(id),
        Msg::JobDetail(d) => {
            if Some(&d.info.id) == app.job_id.as_ref() {
                app.job = Some(*d);
            }
        }
        Msg::Info(s) => {
            app.status = Some(s);
            // a pulled hardware file changes the workspace's hardware line
            if let (Some(root), Some(st)) = (&app.root, &app.state) {
                let pull = st.hardware_pulls.get(&root.to_string_lossy().to_string()).cloned();
                app.hw = Some(hardware::status(root, pull.as_ref()));
            }
        }
        Msg::Error(e) => app.status = Some(e),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let store = Arc::new(StateStore::load(StateStore::default_path())?);
    let jobs = JobManager::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<Msg>();
    let mut app = App::new(store, jobs.clone(), tx.clone());
    app.refresh_state();
    // job events → messages
    {
        let mut sub = jobs.subscribe();
        let tx = tx.clone();
        tokio::spawn(async move {
            while let Ok(ev) = sub.recv().await {
                if tx.send(Msg::Job(ev)).is_err() {
                    break;
                }
            }
        });
    }
    // optional: open a folder given on the command line
    if let Some(p) = std::env::args().nth(1) {
        app.open(p);
    }

    terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, terminal::EnterAlternateScreen)?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut term = Terminal::new(backend)?;
    let mut events = EventStream::new();
    let mut titles: Titles = HashMap::new();
    let mut dirty = true;
    let result: Result<()> = async {
        loop {
            if dirty {
                term.draw(|f| draw(f, &mut app, &titles))?;
                dirty = false;
            }
            tokio::select! {
                ev = events.next() => {
                    match ev {
                        Some(Ok(Event::Key(k))) if k.kind != crossterm::event::KeyEventKind::Release => {
                            app.status = None;
                            if !handle_key(&mut app, k) { break; }
                            dirty = true;
                        }
                        Some(Ok(Event::Resize(..))) => dirty = true,
                        Some(Err(e)) => { app.status = Some(e.to_string()); dirty = true; }
                        None => break,
                        _ => {}
                    }
                }
                msg = rx.recv() => {
                    let Some(msg) = msg else { break };
                    debug_log(&msg);
                    // the state refresh after a save must not wipe a pulled-hardware notice
                    apply_msg(&mut app, msg, &mut titles);
                    dirty = true;
                }
            }
        }
        Ok(())
    }
    .await;
    terminal::disable_raw_mode()?;
    execute!(term.backend_mut(), terminal::LeaveAlternateScreen)?;
    result
}
