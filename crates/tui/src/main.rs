//! Terminal UI over the Nixos-manager core, for machines without a browser
//! (headless servers, a PTTY over SSH). It links the core library directly:
//! no daemon is needed, jobs run in-process and stream into the log view.
//!
//! Screens: pick a folder → workspace (summary, host, targets) → build
//! dialog / validate / generations → job log (live, stoppable).

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::{execute, terminal};
use futures::StreamExt;
use nixos_manager_core::jobs::JobManager;
use nixos_manager_core::state::StateStore;
use nixos_manager_core::types::{
    AfterBuild, AppState, BuildStatus, ConfigOverview, DeployRequest, DeployTarget, Generation, HardwareChoice, JobDetail, JobEvent, JobStatus, KeepConfiguration,
    LocationSuggestion, LogStream, RebuildAction, RemoteCheck,
};
use nixos_manager_core::{deploy, discover, fsbrowse, hardware, nix, remote, system, validate};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

// ───────────────────────── messages from background work ─────────────────────────

enum Msg {
    State(AppState),
    Suggestions(Vec<LocationSuggestion>),
    Opened(Result<(PathBuf, ConfigOverview, Option<BuildStatus>), String>),
    Check(Result<RemoteCheck, String>),
    Generations(Result<Vec<Generation>, String>),
    Job(JobEvent),
    Error(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Pick,
    Workspace,
    Build,
    Generations,
    Job,
    Jobs,
}

/// One row of the build form.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    DeployTo,
    Mode,
    Host,
    Action,
    Upgrade,
    SkipSync,
    After,
    LocalPassword,
    TargetPassword,
    Start,
}

struct BuildForm {
    cursor: usize,
    target_idx: usize, // 0 = this machine
    on_target: bool,
    host_idx: usize,
    action: RebuildAction,
    upgrade: bool,
    skip_sync: bool,
    after: Option<AfterBuild>,
    local_password: String,
    target_password: String,
    check: Option<Result<RemoteCheck, String>>,
    checking: bool,
}

impl BuildForm {
    fn new() -> Self {
        Self { cursor: 0, target_idx: 0, on_target: false, host_idx: 0, action: RebuildAction::Switch, upgrade: false, skip_sync: false, after: None, local_password: String::new(), target_password: String::new(), check: None, checking: false }
    }
    fn fields(&self, has_targets: bool, hosts: usize) -> Vec<Field> {
        let mut f = Vec::new();
        if has_targets {
            f.push(Field::DeployTo);
        }
        if self.target_idx > 0 {
            f.push(Field::Mode);
        }
        if hosts > 1 {
            f.push(Field::Host);
        }
        f.push(Field::Action);
        f.push(Field::Upgrade);
        if self.target_idx == 0 {
            f.push(Field::SkipSync);
        }
        f.push(Field::After);
        if self.target_idx == 0 {
            f.push(Field::LocalPassword);
        } else {
            f.push(Field::TargetPassword);
        }
        f.push(Field::Start);
        f
    }
}

struct App {
    screen: Screen,
    store: Arc<StateStore>,
    jobs: JobManager,
    tx: mpsc::UnboundedSender<Msg>,
    status: Option<String>,
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
    host: Option<String>,
    targets: Vec<DeployTarget>,
    // build
    form: BuildForm,
    // generations
    generations: Vec<Generation>,
    gen_list: ListState,
    gen_password: String,
    gen_prompt: Option<u32>,
    // jobs
    job_id: Option<String>,
    job: Option<JobDetail>,
    job_scroll: usize,
    job_follow: bool,
    job_ids: Vec<String>,
    jobs_list: ListState,
}

impl App {
    fn new(store: Arc<StateStore>, jobs: JobManager, tx: mpsc::UnboundedSender<Msg>) -> Self {
        Self {
            screen: Screen::Pick, store, jobs, tx, status: None, state: None, suggestions: vec![], pick_list: ListState::default().with_selected(Some(0)), pick_input: String::new(), pick_input_focus: false,
            root: None, overview: None, build_status: None, host: None, targets: vec![], form: BuildForm::new(), generations: vec![], gen_list: ListState::default().with_selected(Some(0)), gen_password: String::new(), gen_prompt: None,
            job_id: None, job: None, job_scroll: 0, job_follow: true, job_ids: vec![], jobs_list: ListState::default().with_selected(Some(0)),
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
            let res: Result<(PathBuf, ConfigOverview, Option<BuildStatus>), String> = async {
                let root = fsbrowse::config_root(&path).map_err(|e| e.to_string())?;
                // same housekeeping as the web UI on open
                let pull = store.get().await.hardware_pulls.get(&root.to_string_lossy().to_string()).cloned();
                let _ = hardware::sync_from_system(&root, pull.as_ref());
                let _ = nixos_manager_core::flake::remove_workspace_lock(&root);
                let ov = discover::overview(&root).await.map_err(|e| e.to_string())?;
                store.touch_location(&root.to_string_lossy(), ov.location.hosts.first().cloned()).await.map_err(|e| e.to_string())?;
                let r2 = root.clone();
                let bs = tokio::task::spawn_blocking(move || JobManager::build_status(&r2)).await.ok();
                Ok((root, ov, bs))
            }
            .await;
            let _ = tx.send(Msg::Opened(res));
        });
    }

    fn start_build(&mut self) {
        let Some(root) = self.root.clone() else { return };
        let target = if self.form.target_idx > 0 { self.targets.get(self.form.target_idx - 1).cloned() } else { None };
        let hosts = self.overview.as_ref().map(|o| o.location.hosts.clone()).unwrap_or_default();
        let host = hosts.get(self.form.host_idx).cloned().or(self.host.clone());
        if target.is_none() && self.form.local_password.is_empty() {
            self.status = Some("Enter your sudo password first.".into());
            return;
        }
        let req = DeployRequest {
            path: root.to_string_lossy().to_string(),
            host,
            action: self.form.action,
            upgrade: self.form.upgrade,
            keep_configuration: Some(KeepConfiguration::Local),
            hardware: Some(HardwareChoice::System),
            skip_sync: self.form.skip_sync,
            password: self.form.local_password.clone(),
            secrets_password: None,
            target: target.clone(),
            target_sudo_password: if self.form.target_password.is_empty() { None } else { Some(self.form.target_password.clone()) },
            build_host: None,
            after: self.form.after,
            after_even_on_failure: false,
            after_local: None,
            on_target: target.is_some() && self.form.on_target,
            allow_unfree_env: target.is_some() && self.form.on_target,
        };
        let title = match &target {
            Some(t) if req.on_target => format!("nixos-rebuild {:?} on {} ({})", req.action, t.name, req.path),
            Some(t) => format!("nixos-rebuild {:?} → {} ({})", req.action, t.name, req.path),
            None => format!("nixos-rebuild {:?} ({})", req.action, req.path),
        };
        let jobs = self.jobs.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let job = jobs.create("deploy", &title).await;
            let id = job.id.clone();
            tokio::spawn(deploy::run(job, req));
            let _ = tx.send(Msg::Error(format!("__open_job:{id}")));
        });
        self.form.local_password.clear();
        self.form.target_password.clear();
    }

    fn start_validate(&mut self) {
        let Some(root) = self.root.clone() else { return };
        let jobs = self.jobs.clone();
        let tx = self.tx.clone();
        let host = self.host.clone();
        tokio::spawn(async move {
            let job = jobs.create("validate", &format!("Validate {}", root.display())).await;
            let id = job.id.clone();
            tokio::spawn(validate::run(job, root, host));
            let _ = tx.send(Msg::Error(format!("__open_job:{id}")));
        });
    }

    fn load_generations(&mut self) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Msg::Generations(nix::generations().await.map_err(|e| e.to_string())));
        });
    }

    fn run_rollback(&mut self, number: u32) {
        let pw = std::mem::take(&mut self.gen_password);
        if pw.is_empty() {
            self.status = Some("Enter your sudo password first.".into());
            return;
        }
        let jobs = self.jobs.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let job = jobs.create("system", &format!("Roll back to generation {number}")).await;
            let id = job.id.clone();
            tokio::spawn(system::rollback(job, number, pw));
            let _ = tx.send(Msg::Error(format!("__open_job:{id}")));
        });
    }

    fn check_target(&mut self) {
        if self.form.target_idx == 0 {
            self.form.check = None;
            return;
        }
        let Some(t) = self.targets.get(self.form.target_idx - 1).cloned() else { return };
        self.form.checking = true;
        self.form.check = None;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let name = t.name.clone();
            let c = remote::check(&t).await;
            let _ = name; let _ = tx.send(Msg::Check(Ok(c)));
        });
    }

    fn open_job(&mut self, id: String) {
        self.job_id = Some(id.clone());
        self.job = None;
        self.job_scroll = 0;
        self.job_follow = true;
        if !self.job_ids.contains(&id) {
            self.job_ids.insert(0, id.clone());
        }
        self.screen = Screen::Job;
        let jobs = self.jobs.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Some(d) = jobs.get(&id).await {
                let _ = tx.send(Msg::Job(JobEvent::Created { job: d.info.clone() }));
                // pull the full detail once; later lines arrive as events
                let _ = tx.send(Msg::Error(format!("__job_detail:{}", id)));
            }
        });
    }

    fn refresh_job(&mut self) {
        let Some(id) = self.job_id.clone() else { return };
        let jobs = self.jobs.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Some(d) = jobs.get(&id).await {
                let _ = tx.send(Msg::Error(format!("__job_json:{}", serde_json_detail(&d))));
            }
        });
    }
}

fn serde_json_detail(d: &JobDetail) -> String {
    // the detail is moved through the message channel as JSON to keep Msg simple
    serde_json::to_string(d).unwrap_or_default()
}

// ───────────────────────── key handling ─────────────────────────

fn handle_key(app: &mut App, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return false;
    }
    match app.screen {
        Screen::Pick => key_pick(app, key),
        Screen::Workspace => key_workspace(app, key),
        Screen::Build => key_build(app, key),
        Screen::Generations => key_generations(app, key),
        Screen::Job => key_job(app, key),
        Screen::Jobs => key_jobs(app, key),
    }
}

fn key_pick(app: &mut App, key: KeyEvent) -> bool {
    let n = app.pick_entries().len();
    match key.code {
        KeyCode::Char('q') if !app.pick_input_focus => return false,
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
        KeyCode::Char('g') => {
            app.gen_prompt = None;
            app.gen_password.clear();
            app.load_generations();
            app.screen = Screen::Generations;
        }
        KeyCode::Char('j') => {
            app.screen = Screen::Jobs;
        }
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
    match key.code {
        KeyCode::Esc => app.screen = Screen::Workspace,
        KeyCode::Up => app.form.cursor = app.form.cursor.saturating_sub(1),
        KeyCode::Down | KeyCode::Tab => app.form.cursor = (app.form.cursor + 1).min(fields.len() - 1),
        KeyCode::Enter if cur == Field::Start => {
            app.start_build();
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Left | KeyCode::Char(' ') if !typing => {
            let fwd = key.code != KeyCode::Left;
            match cur {
                Field::DeployTo => {
                    let n = app.targets.len() + 1;
                    app.form.target_idx = if fwd { (app.form.target_idx + 1) % n } else { (app.form.target_idx + n - 1) % n };
                    app.check_target();
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
                Field::After => {
                    app.form.after = match app.form.after {
                        None => Some(AfterBuild::Reboot),
                        Some(AfterBuild::Reboot) => Some(AfterBuild::Poweroff),
                        _ => None,
                    }
                }
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

fn key_generations(app: &mut App, key: KeyEvent) -> bool {
    if let Some(n) = app.gen_prompt {
        match key.code {
            KeyCode::Esc => {
                app.gen_prompt = None;
                app.gen_password.clear();
            }
            KeyCode::Enter => {
                app.gen_prompt = None;
                app.run_rollback(n);
            }
            KeyCode::Backspace => {
                app.gen_password.pop();
            }
            KeyCode::Char(c) => app.gen_password.push(c),
            _ => {}
        }
        return true;
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.screen = Screen::Workspace,
        KeyCode::Up => {
            let i = app.gen_list.selected().unwrap_or(0);
            app.gen_list.select(Some(i.saturating_sub(1)));
        }
        KeyCode::Down => {
            let i = app.gen_list.selected().unwrap_or(0);
            if !app.generations.is_empty() {
                app.gen_list.select(Some((i + 1).min(app.generations.len() - 1)));
            }
        }
        KeyCode::Enter => {
            if let Some(g) = app.gen_list.selected().and_then(|i| app.generations.get(i)) {
                if g.current {
                    app.status = Some("That generation is already the current one.".into());
                } else {
                    app.gen_prompt = Some(g.number);
                }
            }
        }
        KeyCode::Char('r') => app.load_generations(),
        _ => {}
    }
    true
}

fn key_job(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.screen = Screen::Workspace,
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
        KeyCode::Esc | KeyCode::Char('q') => app.screen = Screen::Workspace,
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

fn draw(f: &mut Frame, app: &mut App, titles: &std::collections::HashMap<String, (String, JobStatus)>) {
    let area = f.area();
    let chunks = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(3), Constraint::Length(1)]).split(area);
    match app.screen {
        Screen::Pick => draw_pick(f, app, chunks[0]),
        Screen::Workspace => draw_workspace(f, app, chunks[0]),
        Screen::Build => draw_build(f, app, chunks[0]),
        Screen::Generations => draw_generations(f, app, chunks[0]),
        Screen::Job => draw_job(f, app, chunks[0]),
        Screen::Jobs => draw_jobs(f, app, chunks[0], titles),
    }
    let help = match app.screen {
        Screen::Pick => " ↑↓ choose · Enter open · / or Tab type a path · q quit ",
        Screen::Workspace => " b build · v validate · g generations · j jobs · h next host · r rescan · q back ",
        Screen::Build => " ↑↓ field · ←→/Space change · type passwords · Enter on Start · Esc back ",
        Screen::Generations => " ↑↓ choose · Enter roll back · r reload · Esc back ",
        Screen::Job => " s stop · ↑↓ PgUp PgDn scroll · f follow · j jobs · Esc back ",
        Screen::Jobs => " ↑↓ choose · Enter open · Esc back ",
    };
    let status = app.status.clone().unwrap_or_default();
    let line = Line::from(vec![Span::styled(help, Style::default().fg(Color::Black).bg(Color::Cyan)), Span::raw(" "), Span::styled(status, Style::default().fg(Color::Yellow))]);
    f.render_widget(Paragraph::new(line), chunks[1]);
}

fn block(title: &str) -> Block<'_> {
    Block::default().borders(Borders::ALL).title(Span::styled(format!(" {title} "), Style::default().add_modifier(Modifier::BOLD)))
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
    let mut lines: Vec<Line> = vec![
        Line::from(vec![Span::styled("Folder   ", Style::default().fg(Color::DarkGray)), Span::raw(ov.location.path.clone())]),
        Line::from(vec![Span::styled("Type     ", Style::default().fg(Color::DarkGray)), Span::raw(format!("{:?}", ov.location.kind).to_lowercase())]),
        Line::from(vec![Span::styled("Host     ", Style::default().fg(Color::DarkGray)), Span::styled(app.host.clone().unwrap_or_else(|| "-".into()), Style::default().fg(Color::Cyan)), Span::styled(if ov.location.hosts.len() > 1 { format!("  (h cycles: {})", ov.location.hosts.join(", ")) } else { String::new() }, Style::default().fg(Color::DarkGray))]),
        Line::from(vec![Span::styled("Machine  ", Style::default().fg(Color::DarkGray)), Span::raw(format!("{} · NixOS {}", ov.machine.hostname.clone().unwrap_or_default(), ov.machine.nixos_version.clone().unwrap_or_default()))]),
        Line::from(vec![Span::styled("Writable ", Style::default().fg(Color::DarkGray)), Span::raw(if ov.location.writable { "yes" } else { "no" })]),
    ];
    let broken = ov.files.iter().filter(|x| !x.parse_ok).count();
    if broken > 0 {
        lines.push(Line::from(Span::styled(format!("{broken} file(s) have syntax errors"), Style::default().fg(Color::Red))));
    }
    if let Some(bs) = &app.build_status {
        match (&bs.last_build_at, bs.changed_files.len()) {
            (Some(at), 0) => lines.push(Line::from(Span::styled(format!("Last successful build {} · nothing changed since", at.get(..16).unwrap_or(at).replace('T', " ")), Style::default().fg(Color::Green)))),
            (Some(at), n) => lines.push(Line::from(Span::styled(format!("{n} file(s) changed since the last build ({}) — press b to build", at.get(..16).unwrap_or(at).replace('T', " ")), Style::default().fg(Color::Yellow)))),
            (None, _) => lines.push(Line::from(Span::styled("No build recorded yet for this folder", Style::default().fg(Color::DarkGray)))),
        }
    }
    lines.push(Line::raw(""));
    if app.targets.is_empty() {
        lines.push(Line::from(Span::styled("No deploy targets (add them in the web UI, Remote page); builds apply to this machine.", Style::default().fg(Color::DarkGray))));
    } else {
        lines.push(Line::from(Span::styled("Deploy targets:", Style::default().fg(Color::DarkGray))));
        for t in &app.targets {
            lines.push(Line::raw(format!("  {}  {}{}", t.name, t.ssh_target, t.port.map(|p| format!(":{p}")).unwrap_or_default())));
        }
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![Span::styled("b", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), Span::raw(" Build & apply    "), Span::styled("v", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), Span::raw(" Validate (no sudo)    "), Span::styled("g", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), Span::raw(" Generations / roll back    "), Span::styled("j", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)), Span::raw(" Jobs")]));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block("Configuration")), cols[1]);
}

fn draw_build(f: &mut Frame, app: &mut App, area: Rect) {
    let hosts: Vec<String> = app.overview.as_ref().map(|o| o.location.hosts.clone()).unwrap_or_default();
    let fields = app.form.fields(!app.targets.is_empty(), hosts.len());
    let cursor = app.form.cursor.min(fields.len() - 1);
    let target = if app.form.target_idx > 0 { app.targets.get(app.form.target_idx - 1) } else { None };
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
            Field::DeployTo => ("Deploy to", match target { Some(t) => format!("{} ({})", t.name, t.ssh_target), None => "This machine (sync to /etc/nixos, then rebuild)".into() }),
            Field::Mode => ("Mode", if app.form.on_target { "Build and activate on the target (copy the working copy there)".into() } else { "Build here, send the result to the target".into() }),
            Field::Host => ("Host", hosts.get(app.form.host_idx).cloned().unwrap_or_default()),
            Field::Action => ("Action", action_label.into()),
            Field::Upgrade => ("Upgrade", if app.form.upgrade { "yes — update all flake inputs".into() } else { "no — keep the pinned inputs".into() }),
            Field::SkipSync => ("Skip sync", if app.form.skip_sync { "yes — rebuild /etc/nixos as it is".into() } else { "no — copy the working copy to /etc/nixos first".into() }),
            Field::After => ("Afterwards", match app.form.after { None => "stay on".into(), Some(AfterBuild::Reboot) => "reboot".into(), Some(AfterBuild::Poweroff) => "power off".into(), Some(AfterBuild::None) => "stay on".into() }),
            Field::LocalPassword => ("Your sudo password", "•".repeat(app.form.local_password.len())),
            Field::TargetPassword => ("sudo password on the target", if app.form.target_password.is_empty() { String::new() } else { "•".repeat(app.form.target_password.len()) }),
            Field::Start => ("", "▶ Start".into()),
        };
        let style = if sel { Style::default().bg(Color::Blue).fg(Color::White) } else { Style::default() };
        let label_s = if label.is_empty() { String::new() } else { format!("{label:<28}") };
        lines.push(Line::from(vec![Span::styled(if sel { "▶ " } else { "  " }, style), Span::styled(label_s, style.fg(if sel { Color::White } else { Color::DarkGray })), Span::styled(value, style)]));
    }
    lines.push(Line::raw(""));
    if let Some(t) = target {
        match (&app.form.checking, &app.form.check) {
            (true, _) => lines.push(Line::from(Span::styled("Checking the target…", Style::default().fg(Color::Yellow)))),
            (_, Some(Ok(c))) if !c.reachable => lines.push(Line::from(Span::styled(format!("Cannot reach {}: {}", t.ssh_target, c.message.clone().unwrap_or_default()), Style::default().fg(Color::Red)))),
            (_, Some(Ok(c))) => {
                lines.push(Line::raw(format!("Target {} · NixOS {} · trusted by nix: {} · passwordless sudo: {}", c.hostname.clone().unwrap_or_default(), c.nixos_version.clone().unwrap_or_default(), if c.is_root { "root".into() } else { c.trusted.map(|b| if b { "yes" } else { "no" }).unwrap_or("?").to_string() }, if c.is_root { "n/a".into() } else { c.passwordless_sudo.map(|b| if b { "yes" } else { "no" }).unwrap_or("?").to_string() })));
                let need = if app.form.on_target { !c.is_root && c.passwordless_sudo != Some(true) } else { c.needs_sudo_password };
                if need {
                    lines.push(Line::from(Span::styled("The target's sudo password is needed (type it in the field above).", Style::default().fg(Color::Yellow))));
                }
            }
            (_, Some(Err(e))) => lines.push(Line::from(Span::styled(e.clone(), Style::default().fg(Color::Red)))),
            _ => {}
        }
        if !app.form.on_target {
            lines.push(Line::from(Span::styled("Build here, send the result: the target's own hardware-configuration.nix and flake.lock are used; temporary nix trust is granted when needed.", Style::default().fg(Color::DarkGray))));
        } else {
            lines.push(Line::from(Span::styled("On the target: the working copy is copied there, synced to its /etc/nixos and rebuilt as root (flakes and unfree allowed through the environment).", Style::default().fg(Color::DarkGray))));
        }
    } else {
        lines.push(Line::from(Span::styled("Same steps as sync.sh, as root: preserve /etc/nixos/flake.lock, copy hardware-configuration.nix from /etc/nixos, clear /etc/nixos, copy the working copy, nixos-rebuild.", Style::default().fg(Color::DarkGray))));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block("Build & apply")), area);
}

fn draw_generations(f: &mut Frame, app: &mut App, area: Rect) {
    let items: Vec<ListItem> = app.generations.iter().map(|g| {
        let mut spans = vec![Span::styled(format!("#{:<5}", g.number), Style::default().fg(Color::Cyan)), Span::raw(format!("{}  NixOS {}", g.date, g.nixos_version.clone().unwrap_or_default()))];
        if let Some(k) = &g.kernel {
            spans.push(Span::styled(format!("  kernel {k}"), Style::default().fg(Color::DarkGray)));
        }
        if g.current {
            spans.push(Span::styled("  current", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)));
        }
        ListItem::new(Line::from(spans))
    }).collect();
    let list = List::new(items).block(block("Generations — Enter rolls back to the selected one")).highlight_style(Style::default().bg(Color::Blue).fg(Color::White)).highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.gen_list);
    if let Some(n) = app.gen_prompt {
        let w = area.width.min(70);
        let popup = Rect { x: area.x + (area.width.saturating_sub(w)) / 2, y: area.y + area.height / 2 - 3, width: w, height: 6 };
        f.render_widget(Clear, popup);
        let text = vec![
            Line::raw(format!("Roll back to generation #{n}? The system switches to it now and at boot.")),
            Line::raw(""),
            Line::from(vec![Span::raw("sudo password: "), Span::styled("•".repeat(app.gen_password.len()), Style::default().fg(Color::Cyan)), Span::raw("▏")]),
            Line::from(Span::styled("Enter confirm · Esc cancel", Style::default().fg(Color::DarkGray))),
        ];
        f.render_widget(Paragraph::new(text).block(block("Confirm")), popup);
    }
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

fn draw_jobs(f: &mut Frame, app: &mut App, area: Rect, titles: &std::collections::HashMap<String, (String, JobStatus)>) {
    let items: Vec<ListItem> = app.job_ids.iter().map(|id| {
        let (t, s) = titles.get(id).cloned().unwrap_or((id.clone(), JobStatus::Pending));
        let st = match s { JobStatus::Succeeded => Color::Green, JobStatus::Failed => Color::Red, JobStatus::Cancelled => Color::Yellow, _ => Color::Cyan };
        ListItem::new(Line::from(vec![Span::styled(format!("{:<10}", format!("{s:?}").to_lowercase()), Style::default().fg(st)), Span::raw(t)]))
    }).collect();
    let list = List::new(items).block(block("Jobs of this session (older logs: ~/.local/state/nixos-manager/logs)")).highlight_style(Style::default().bg(Color::Blue).fg(Color::White)).highlight_symbol("▶ ");
    f.render_stateful_widget(list, area, &mut app.jobs_list);
}

// ───────────────────────── main loop ─────────────────────────

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
    let mut titles: std::collections::HashMap<String, (String, JobStatus)> = std::collections::HashMap::new();
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
                    if let Ok(f) = std::env::var("NM_TUI_LOG") {
                        use std::io::Write;
                        let what = match &msg { Msg::State(_) => "state".to_string(), Msg::Suggestions(s) => format!("suggestions {}", s.len()), Msg::Opened(r) => format!("opened {:?}", r.as_ref().map(|x| x.0.display().to_string())), Msg::Check(_) => "check".into(), Msg::Generations(r) => format!("generations {:?}", r.as_ref().map(|g| g.len())), Msg::Job(_) => "job".into(), Msg::Error(e) => format!("msg {e}") };
                        if let Ok(mut fh) = std::fs::OpenOptions::new().create(true).append(true).open(f) { let _ = writeln!(fh, "{what}"); }
                    }
                    match msg {
                        Msg::State(st) => { app.targets = st.deploy_targets.clone(); app.state = Some(st); }
                        Msg::Suggestions(s) => app.suggestions = s,
                        Msg::Opened(Ok((root, ov, bs))) => {
                            app.host = ov.location.hosts.iter().find(|h| Some(*h) == ov.machine.hostname.as_ref()).or(ov.location.hosts.first()).cloned();
                            if let Some(st) = &app.state {
                                if let Some(h) = st.recent_locations.iter().find(|r| r.path == root.to_string_lossy()).and_then(|r| r.host.clone()) {
                                    if ov.location.hosts.contains(&h) { app.host = Some(h); }
                                }
                            }
                            app.root = Some(root); app.overview = Some(ov); app.build_status = bs; app.status = None;
                            app.screen = Screen::Workspace;
                        }
                        Msg::Opened(Err(e)) => app.status = Some(e),
                        Msg::Check(c) => { app.form.checking = false; app.form.check = Some(c); }
                        Msg::Generations(Ok(g)) => { app.generations = g; app.gen_list.select(Some(0)); }
                        Msg::Generations(Err(e)) => app.status = Some(e),
                        Msg::Job(ev) => {
                            match &ev {
                                JobEvent::Created { job } => { titles.insert(job.id.clone(), (job.title.clone(), job.status)); if !app.job_ids.contains(&job.id) { app.job_ids.insert(0, job.id.clone()); } }
                                JobEvent::Status { job_id, status, .. } => { if let Some(t) = titles.get_mut(job_id) { t.1 = *status; } }
                                _ => {}
                            }
                            let mine = match &ev { JobEvent::Log { job_id, .. } | JobEvent::Status { job_id, .. } => Some(job_id.clone()), JobEvent::Created { job } => Some(job.id.clone()) };
                            if mine.as_deref() == app.job_id.as_deref() {
                                match ev {
                                    JobEvent::Log { line, .. } => { if let Some(d) = app.job.as_mut() { d.log.push(line); d.info.log_lines = d.log.len() as u32; } else { app.refresh_job(); } }
                                    JobEvent::Status { status, error, .. } => { if let Some(d) = app.job.as_mut() { d.info.status = status; d.info.error = error; } else { app.refresh_job(); } }
                                    JobEvent::Created { .. } => app.refresh_job(),
                                }
                            }
                        }
                        Msg::Error(e) => {
                            if let Some(id) = e.strip_prefix("__open_job:") { app.open_job(id.to_string()); }
                            else if let Some(id) = e.strip_prefix("__job_detail:") { let _ = id; app.refresh_job(); }
                            else if let Some(j) = e.strip_prefix("__job_json:") { if let Ok(d) = serde_json::from_str::<JobDetail>(j) { if Some(&d.info.id) == app.job_id.as_ref() { app.job = Some(d); } } }
                            else { app.status = Some(e); }
                        }
                    }
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
