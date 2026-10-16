//! flake.nix: parsing into [`FlakeInfo`], structured edits (inputs, extra
//! nixpkgs channels, global vars), the template used when converting a
//! classic configuration, and lock-file reading.
//!
//! The template mirrors Vincent's layout: `inputs` with nixpkgs plus optional
//! `nixpkgs-<name>` extras, and an `outputs` let-block holding `vars`,
//! `system` and `channels`, which generates `pkgs-<name>` module arguments.

use crate::edit::{self, add_binding_edit, list_append_edit, remove_item_edit, splice_checked, Edit};
use crate::nixparse::{self, attrpath_text, Parsed, TreeBuilder};
use crate::types::{FlakeFollow, FlakeInfo, FlakeInput, FlakeOp, FlakeVar, LockedInput, NodeKind, PackageSet};
use crate::{CoreError, Result};
use rnix::ast::{self, HasEntry};
use rnix::{SyntaxNode, TextRange};
use rowan::ast::AstNode;
use std::collections::BTreeMap;
use std::path::Path;

pub const DEFAULT_NIXPKGS_REF: &str = "nixos-unstable";

// ───────────────────────── template ─────────────────────────

pub fn template(hostname: &str, username: &str, system: &str, nixpkgs_ref: &str) -> String {
    let host = if hostname.trim().is_empty() { "main" } else { hostname.trim() };
    let user = if username.trim().is_empty() { "user" } else { username.trim() };
    format!(
        r#"{{
  description = "NixOS configuration for {host}";

  # Package sources. `nixpkgs` is the main one (nixos-unstable has the newest
  # packages; a release branch such as nixos-26.05 changes less often).
  # "Add a previous nixpkgs version" in Nixos-manager adds an input named
  # nixpkgs-<name> below and <name> to `channels`; every module then receives
  # `pkgs-<name>` (and `pkgs-<name>-32`) as arguments.
  inputs = {{
    nixpkgs.url = "github:nixos/nixpkgs?ref={nixpkgs_ref}";
  }};

  outputs = inputs@{{ self, nixpkgs, ... }}:
  let
    # Global values every module receives as `vars` (e.g. vars.mainUsername).
    vars = {{
      mainUsername = "{user}";
      mainUserGroup = "users";
      mainUserHome = "/home/{user}";
    }};

    system = "{system}";

    # Names of the extra package sets; each needs an input `nixpkgs-<name>`.
    channels = [ ];

    mkPkgs = input: sys: import input {{
      system = sys;
      config.allowUnfree = true;
    }};

    channelPkgs = builtins.listToAttrs (nixpkgs.lib.concatMap (name: [
      {{ name = "pkgs-${{name}}";    value = mkPkgs inputs."nixpkgs-${{name}}" system; }}
      {{ name = "pkgs-${{name}}-32"; value = mkPkgs inputs."nixpkgs-${{name}}" "i686-linux"; }}
    ]) channels);
  in
  {{
    nixosConfigurations.{host} = nixpkgs.lib.nixosSystem {{
      inherit system;
      specialArgs = {{ inherit inputs system vars; }} // channelPkgs;
      modules = [
        ./configuration.nix
      ];
    }};
  }};
}}
"#
    )
}

/// Create flake.nix in `dir` and make sure configuration.nix enables flakes.
pub fn convert(dir: &Path, hostname: &str, username: &str, nixpkgs_ref: &str) -> Result<()> {
    let flake_path = dir.join("flake.nix");
    if flake_path.exists() {
        return Err(CoreError::Conflict("flake.nix already exists".into()));
    }
    let cfg_path = dir.join("configuration.nix");
    if !cfg_path.is_file() {
        return Err(CoreError::Invalid("configuration.nix not found; a flake needs it as the entry module".into()));
    }
    let system = detect_system();
    let text = template(hostname, username, &system, nixpkgs_ref);
    let parsed = Parsed::parse(text.clone());
    if !parsed.errors.is_empty() {
        return Err(CoreError::Other("generated flake.nix does not parse".into()));
    }
    // Enable flakes in configuration.nix (harmless if already present).
    let cfg = std::fs::read_to_string(&cfg_path)?;
    let cfg = edit::append_to_list_setting(&cfg, "nix.settings.experimental-features", "\"nix-command\"")?;
    let cfg = edit::append_to_list_setting(&cfg, "nix.settings.experimental-features", "\"flakes\"")?;
    edit::write_atomic(&cfg_path, &cfg)?;
    edit::write_atomic(&flake_path, &text)?;
    Ok(())
}

pub fn detect_system() -> String {
    match std::env::consts::ARCH {
        "aarch64" => "aarch64-linux".into(),
        _ => "x86_64-linux".into(),
    }
}

// ───────────────────────── parsing ─────────────────────────

struct FlakeAst {
    parsed: Parsed,
    root_set: Option<ast::AttrSet>,
}

impl FlakeAst {
    fn new(source: String) -> Self {
        let parsed = Parsed::parse(source);
        let root_set = parsed.root.expr().and_then(|e| if let ast::Expr::AttrSet(s) = e { Some(s) } else { None });
        FlakeAst { parsed, root_set }
    }

    fn top_entry(&self, name: &str) -> Option<ast::AttrpathValue> {
        self.root_set.as_ref()?.attrpath_values().find(|av| av.attrpath().map(|p| attrpath_text(&p) == name).unwrap_or(false))
    }

    /// The `inputs = { ... }` attrset, if written that way.
    fn inputs_set(&self) -> Option<ast::AttrSet> {
        match self.top_entry("inputs")?.value()? {
            ast::Expr::AttrSet(s) => Some(s),
            _ => None,
        }
    }

    /// The let-block of `outputs = ...: let ... in ...`.
    fn outputs_let(&self) -> Option<ast::LetIn> {
        let mut expr = self.top_entry("outputs")?.value()?;
        loop {
            match expr {
                ast::Expr::Lambda(l) => expr = l.body()?,
                ast::Expr::LetIn(li) => return Some(li),
                ast::Expr::Paren(p) => expr = p.expr()?,
                _ => return None,
            }
        }
    }

    fn let_entry(&self, name: &str) -> Option<ast::AttrpathValue> {
        self.outputs_let()?.attrpath_values().find(|av| av.attrpath().map(|p| attrpath_text(&p) == name).unwrap_or(false))
    }

    fn vars_set(&self) -> Option<ast::AttrSet> {
        match self.let_entry("vars")?.value()? {
            ast::Expr::AttrSet(s) => Some(s),
            _ => None,
        }
    }

    fn channels_list(&self) -> Option<ast::List> {
        match self.let_entry("channels")?.value()? {
            ast::Expr::List(l) => Some(l),
            _ => None,
        }
    }
}

fn str_value(expr: &ast::Expr) -> Option<String> {
    if let ast::Expr::Str(s) = expr {
        let mut out = String::new();
        for part in s.normalized_parts() {
            match part {
                ast::InterpolPart::Literal(t) => out.push_str(&t),
                ast::InterpolPart::Interpolation(_) => return None,
            }
        }
        Some(out)
    } else {
        None
    }
}

/// Flatten `a.b = v; c = { d = w; }` into [("a.b", v), ("c.d", w)].
fn flatten(set: &ast::AttrSet, prefix: &str, out: &mut Vec<(String, ast::Expr)>) {
    for av in set.attrpath_values() {
        let Some(p) = av.attrpath() else { continue };
        let key = if prefix.is_empty() { attrpath_text(&p) } else { format!("{prefix}.{}", attrpath_text(&p)) };
        match av.value() {
            Some(ast::Expr::AttrSet(inner)) => flatten(&inner, &key, out),
            Some(v) => out.push((key, v)),
            None => {}
        }
    }
}

pub fn info(dir: &Path) -> Result<FlakeInfo> {
    let path = dir.join("flake.nix");
    let source = std::fs::read_to_string(&path).map_err(|_| CoreError::NotFound("flake.nix not found".into()))?;
    let (lock_path, lock) = read_lock(dir);
    Ok(info_from_source(source, lock_path, lock))
}

pub fn info_from_source(source: String, lock_path: Option<String>, lock: BTreeMap<String, LockedInput>) -> FlakeInfo {
    let f = FlakeAst::new(source);
    let version = f.parsed.version();
    let description = f.top_entry("description").and_then(|e| e.value()).and_then(|v| str_value(&v));

    // Inputs: both `inputs = { name.url = ..; name = { url = ..; inputs.x.follows = ..; }; }`
    // and top-level `inputs.name.url = ..`.
    let mut inputs: BTreeMap<String, FlakeInput> = BTreeMap::new();
    let mut leaves: Vec<(String, ast::Expr)> = Vec::new();
    if let Some(set) = f.inputs_set() {
        flatten(&set, "", &mut leaves);
    }
    if let Some(root) = &f.root_set {
        for av in root.attrpath_values() {
            let Some(p) = av.attrpath() else { continue };
            let key = attrpath_text(&p);
            if let Some(rest) = key.strip_prefix("inputs.") {
                match av.value() {
                    Some(ast::Expr::AttrSet(inner)) => flatten(&inner, rest, &mut leaves),
                    Some(v) => leaves.push((rest.to_string(), v)),
                    None => {}
                }
            }
        }
    }
    for (key, value) in leaves {
        let mut parts = key.split('.');
        let Some(name) = parts.next() else { continue };
        let rest: Vec<&str> = parts.collect();
        let entry = inputs.entry(name.to_string()).or_insert_with(|| FlakeInput {
            name: name.to_string(),
            url: None,
            follows: vec![],
            locked: None,
            is_nixpkgs_channel: name.starts_with("nixpkgs-"),
        });
        match rest.as_slice() {
            ["url"] => entry.url = str_value(&value),
            ["inputs", inner, "follows"] => {
                if let Some(t) = str_value(&value) {
                    entry.follows.push(FlakeFollow { input: inner.to_string(), target: t });
                }
            }
            ["follows"] => {
                if let Some(t) = str_value(&value) {
                    entry.follows.push(FlakeFollow { input: "(self)".into(), target: t });
                }
            }
            _ => {}
        }
    }
    // Attach lock data only when the lock was taken from the same source the
    // flake declares (the /etc/nixos fallback may belong to another config).
    for (name, locked) in lock {
        if let Some(i) = inputs.get_mut(&name) {
            if lock_matches(i.url.as_deref(), locked.source.as_deref()) {
                i.locked = Some(locked);
            }
        }
    }

    let has_let = f.outputs_let().is_some();
    let vars = f.vars_set().map(|set| {
        let builder = TreeBuilder::new(&f.parsed);
        set.attrpath_values()
            .filter_map(|av| {
                let name = av.attrpath().map(|p| attrpath_text(&p))?;
                let v = av.value()?;
                let node = builder.convert(&v, None, None, None);
                Some(FlakeVar { name, value: v.syntax().text().to_string(), kind: node.kind })
            })
            .collect()
    });
    let channels = f.channels_list().map(|l| l.items().filter_map(|i| str_value(&i)).collect::<Vec<_>>());
    let system = f.let_entry("system").and_then(|e| e.value()).and_then(|v| str_value(&v));
    let hosts = nixparse::flake_hosts_static(&f.parsed);
    let _ = NodeKind::Code;
    FlakeInfo {
        version,
        description,
        inputs: inputs.into_values().collect(),
        vars,
        channels,
        system,
        hosts,
        has_template_structure: has_let && f.channels_list().is_some() && f.vars_set().is_some(),
        lock_path,
        parse_errors: f.parsed.parse_errors(),
    }
}

/// flake.lock from the working copy, else /etc/nixos (where sync.sh keeps it).
pub fn read_lock(dir: &Path) -> (Option<String>, BTreeMap<String, LockedInput>) {
    for candidate in [dir.join("flake.lock"), Path::new(crate::deploy::ETC_NIXOS).join("flake.lock")] {
        if let Ok(text) = std::fs::read_to_string(&candidate) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                return (Some(candidate.to_string_lossy().to_string()), parse_lock(&v));
            }
        }
    }
    (None, BTreeMap::new())
}

fn parse_lock(v: &serde_json::Value) -> BTreeMap<String, LockedInput> {
    let mut out = BTreeMap::new();
    let nodes = v.get("nodes").and_then(|n| n.as_object());
    let Some(nodes) = nodes else { return out };
    let root_inputs = nodes.get("root").and_then(|r| r.get("inputs")).and_then(|i| i.as_object());
    let Some(root_inputs) = root_inputs else { return out };
    for (name, node_ref) in root_inputs {
        let key = node_ref.as_str().unwrap_or(name);
        let Some(node) = nodes.get(key) else { continue };
        let locked = node.get("locked");
        let original = node.get("original");
        let rev = locked.and_then(|l| l.get("rev")).and_then(|r| r.as_str()).map(|s| s.to_string());
        let date = locked
            .and_then(|l| l.get("lastModified"))
            .and_then(|t| t.as_i64())
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
            .map(|d| d.to_rfc3339());
        let source = original.or(locked).map(|o| {
            let g = |k: &str| o.get(k).and_then(|x| x.as_str()).unwrap_or("");
            match g("type") {
                "github" | "gitlab" | "sourcehut" => {
                    let r = g("ref");
                    if r.is_empty() { format!("{}:{}/{}", g("type"), g("owner"), g("repo")) } else { format!("{}:{}/{}/{}", g("type"), g("owner"), g("repo"), r) }
                }
                "git" | "tarball" | "path" | "indirect" => {
                    let u = g("url");
                    if u.is_empty() { g("id").to_string() } else { u.to_string() }
                }
                _ => serde_json::to_string(o).unwrap_or_default(),
            }
        });
        out.insert(name.clone(), LockedInput { rev, date, source });
    }
    out
}

// ───────────────────────── edits ─────────────────────────

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(CoreError::Invalid(format!("'{name}' is not a valid name (letters, digits, - and _)")));
    }
    Ok(())
}

fn reparse_ok(source: &str) -> Result<()> {
    let p = rnix::Root::parse(source);
    if !p.errors().is_empty() {
        return Err(CoreError::Parse("flake.nix has syntax errors; fix them in code view first".into()));
    }
    Ok(())
}

pub fn apply(source: &str, op: &FlakeOp) -> Result<String> {
    reparse_ok(source)?;
    match op {
        FlakeOp::AddInput { name, url, follows_nixpkgs } => add_input(source, name, url, *follows_nixpkgs),
        FlakeOp::RemoveInput { name } => remove_input(source, name),
        FlakeOp::SetInputUrl { name, url } => {
            validate_name(name)?;
            edit::upsert_setting(source, &format!("inputs.{name}.url"), &edit::nix_string(url))
        }
        FlakeOp::AddChannel { name, url } => {
            validate_name(name)?;
            let src = add_input(source, &format!("nixpkgs-{name}"), url, false)?;
            add_channel_entry(&src, name)
        }
        FlakeOp::RemoveChannel { name } => {
            let src = remove_channel_entry(source, name)?;
            remove_input(&src, &format!("nixpkgs-{name}"))
        }
        FlakeOp::SetVar { name, value } => set_var(source, name, value),
        FlakeOp::RemoveVar { name } => remove_var(source, name),
    }
}

fn add_input(source: &str, name: &str, url: &str, follows_nixpkgs: bool) -> Result<String> {
    validate_name(name)?;
    if url.trim().is_empty() {
        return Err(CoreError::Invalid("url cannot be empty".into()));
    }
    let f = FlakeAst::new(source.to_string());
    if let Some(set) = f.inputs_set() {
        if f.parsed.module_body().2.is_some() {
            // Prefer the nested form when the inputs attrset exists.
            let value = if follows_nixpkgs {
                format!("{{\n      url = {};\n      inputs.nixpkgs.follows = \"nixpkgs\";\n    }}", edit::nix_string(url))
            } else {
                format!("{{\n      url = {};\n    }}", edit::nix_string(url))
            };
            let exists = set.attrpath_values().any(|av| av.attrpath().map(|p| attrpath_text(&p).split('.').next() == Some(name)).unwrap_or(false));
            if exists {
                return Err(CoreError::Conflict(format!("input '{name}' already exists")));
            }
            let e = add_binding_edit(source, set.syntax(), name, &value)?;
            return splice_checked(source, e);
        }
    }
    let src = edit::upsert_setting(source, &format!("inputs.{name}.url"), &edit::nix_string(url))?;
    if follows_nixpkgs {
        edit::upsert_setting(&src, &format!("inputs.{name}.inputs.nixpkgs.follows"), "\"nixpkgs\"")
    } else {
        Ok(src)
    }
}

fn remove_input(source: &str, name: &str) -> Result<String> {
    let f = FlakeAst::new(source.to_string());
    let mut ranges: Vec<TextRange> = Vec::new();
    if let Some(set) = f.inputs_set() {
        for av in set.attrpath_values() {
            if av.attrpath().map(|p| attrpath_text(&p).split('.').next() == Some(name)).unwrap_or(false) {
                ranges.push(av.syntax().text_range());
            }
        }
    }
    if let Some(root) = &f.root_set {
        for av in root.attrpath_values() {
            let key = av.attrpath().map(|p| attrpath_text(&p)).unwrap_or_default();
            if key == format!("inputs.{name}") || key.starts_with(&format!("inputs.{name}.")) {
                ranges.push(av.syntax().text_range());
            }
        }
    }
    if ranges.is_empty() {
        return Err(CoreError::NotFound(format!("input '{name}' not found")));
    }
    ranges.sort_by(|a, b| b.start().cmp(&a.start()));
    let mut src = source.to_string();
    for r in ranges {
        let e = remove_item_edit(&src, r.start().into(), r.end().into());
        src = splice_checked(&src, e)?;
    }
    Ok(src)
}

fn add_channel_entry(source: &str, name: &str) -> Result<String> {
    let f = FlakeAst::new(source.to_string());
    let Some(list) = f.channels_list() else {
        return Err(CoreError::Conflict("this flake has no `channels` list in its outputs let-block; the input was added but modules will not get a pkgs-<name> argument automatically".into()));
    };
    if list.items().any(|i| str_value(&i).as_deref() == Some(name)) {
        return Ok(source.to_string());
    }
    let e = list_append_edit(source, &list, &edit::nix_string(name))?;
    splice_checked(source, e)
}

fn remove_channel_entry(source: &str, name: &str) -> Result<String> {
    let f = FlakeAst::new(source.to_string());
    let Some(list) = f.channels_list() else { return Ok(source.to_string()) };
    let Some(item) = list.items().find(|i| str_value(i).as_deref() == Some(name)) else { return Ok(source.to_string()) };
    let r = item.syntax().text_range();
    let e = remove_item_edit(source, r.start().into(), r.end().into());
    splice_checked(source, e)
}

fn set_var(source: &str, name: &str, value: &str) -> Result<String> {
    validate_name(name)?;
    edit::validate_expr(value)?;
    let f = FlakeAst::new(source.to_string());
    if let Some(set) = f.vars_set() {
        if let Some(av) = set.attrpath_values().find(|av| av.attrpath().map(|p| attrpath_text(&p) == name).unwrap_or(false)) {
            let r = av.value().map(|v| v.syntax().text_range()).ok_or_else(|| CoreError::Other("var has no value".into()))?;
            return splice_checked(source, Edit { range: r, text: value.to_string() });
        }
        let e = add_binding_edit(source, set.syntax(), name, value)?;
        return splice_checked(source, e);
    }
    // No vars yet: add `vars = { name = value; };` to the let-block.
    let Some(li) = f.outputs_let() else {
        return Err(CoreError::Conflict("this flake's outputs has no let-block; add vars by hand in code view".into()));
    };
    let entries: Vec<SyntaxNode> = li.entries().map(|e| e.syntax().clone()).collect();
    let (insert_at, indent) = if let Some(last) = entries.last() {
        let end: usize = last.text_range().end().into();
        let line_end = source[end..].find('\n').map(|i| end + i).unwrap_or(source.len());
        let ls = source[..last.text_range().start().into()].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let indent: String = source[ls..].chars().take_while(|c| *c == ' ').collect();
        (line_end, indent)
    } else {
        let let_tok: usize = li.let_token().map(|t| t.text_range().end().into()).unwrap_or(li.syntax().text_range().start().into());
        (let_tok, "    ".to_string())
    };
    let text = format!("\n{indent}vars = {{\n{indent}  {name} = {value};\n{indent}}};");
    splice_checked(source, Edit { range: TextRange::new((insert_at as u32).into(), (insert_at as u32).into()), text })
}

fn remove_var(source: &str, name: &str) -> Result<String> {
    let f = FlakeAst::new(source.to_string());
    let set = f.vars_set().ok_or_else(|| CoreError::NotFound("no vars block".into()))?;
    let av = set
        .attrpath_values()
        .find(|av| av.attrpath().map(|p| attrpath_text(&p) == name).unwrap_or(false))
        .ok_or_else(|| CoreError::NotFound(format!("var '{name}' not found")))?;
    let r = av.syntax().text_range();
    let e = remove_item_edit(source, r.start().into(), r.end().into());
    splice_checked(source, e)
}

/// Package sets available to modules. Classic configs: just `pkgs`.
/// Flakes: `pkgs` from the nixpkgs input plus `pkgs-<name>` (and `-32`) for
/// each extra channel.
pub fn package_sets(dir: &Path) -> Vec<PackageSet> {
    let Ok(info) = info(dir) else {
        let v = crate::channels::system_nixos_version();
        return vec![PackageSet {
            name: "pkgs".into(),
            label: format!("main · nixos channel{}", v.map(|x| format!(" ({x})")).unwrap_or_default()),
            input: None,
            flake_ref: None,
            is_default: true,
            bits32: false,
        }];
    };
    let mut out = Vec::new();
    let describe = |inp: &FlakeInput| -> (String, Option<String>) {
        let branch = inp.url.as_deref().and_then(branch_of).unwrap_or_else(|| inp.url.clone().unwrap_or_default());
        let date = inp.locked.as_ref().and_then(|l| l.date.clone()).map(|d| d[..10.min(d.len())].to_string());
        let label = match date {
            Some(d) => format!("{branch} ({d})"),
            None => branch,
        };
        (label, flake_ref_for(inp))
    };
    if let Some(main) = info.inputs.iter().find(|i| i.name == "nixpkgs") {
        let (label, fref) = describe(main);
        out.push(PackageSet { name: "pkgs".into(), label: format!("main · {label}"), input: Some("nixpkgs".into()), flake_ref: fref, is_default: true, bits32: false });
    } else {
        out.push(PackageSet { name: "pkgs".into(), label: "main".into(), input: None, flake_ref: None, is_default: true, bits32: false });
    }
    for ch in info.channels.clone().unwrap_or_default() {
        let input_name = format!("nixpkgs-{ch}");
        let (label, fref) = match info.inputs.iter().find(|i| i.name == input_name) {
            Some(i) => describe(i),
            None => (format!("{input_name} (input missing!)"), None),
        };
        out.push(PackageSet { name: format!("pkgs-{ch}"), label: label.clone(), input: Some(input_name.clone()), flake_ref: fref.clone(), is_default: false, bits32: false });
        out.push(PackageSet { name: format!("pkgs-{ch}-32"), label: format!("{label} · 32-bit"), input: Some(input_name), flake_ref: fref, is_default: false, bits32: true });
    }
    out
}

/// Does a lock entry's original source describe the same repo and branch as
/// the declared url? Unknown sources are accepted.
fn lock_matches(url: Option<&str>, source: Option<&str>) -> bool {
    let (Some(url), Some(source)) = (url, source) else { return true };
    let repo = |s: &str| -> String {
        let body = s.split(':').nth(1).unwrap_or(s);
        let body = body.split('?').next().unwrap_or(body);
        body.split('/').take(2).collect::<Vec<_>>().join("/").to_lowercase()
    };
    if repo(url) != repo(source) {
        return false;
    }
    match (branch_of(url), branch_of(source)) {
        (Some(a), Some(b)) => a == b,
        (None, None) => true,
        _ => false,
    }
}

/// "github:nixos/nixpkgs?ref=nixos-25.11" -> "nixos-25.11".
fn branch_of(url: &str) -> Option<String> {
    if let Some(r) = url.split("ref=").nth(1) {
        return Some(r.split('&').next().unwrap_or(r).to_string());
    }
    // github:owner/repo/branch
    let path = url.split(':').nth(1)?;
    let mut parts = path.split('/');
    let (_o, _r, b) = (parts.next()?, parts.next()?, parts.next()?);
    Some(b.split('?').next().unwrap_or(b).to_string())
}

/// Installable for `nix search`: the locked revision when known (exact,
/// already in the store), else the declared url.
fn flake_ref_for(inp: &FlakeInput) -> Option<String> {
    if let (Some(l), Some(url)) = (&inp.locked, &inp.url) {
        if let Some(rev) = &l.rev {
            if url.starts_with("github:") {
                let path = url.trim_start_matches("github:").split('?').next().unwrap_or("");
                let mut it = path.split('/');
                if let (Some(o), Some(r)) = (it.next(), it.next()) {
                    return Some(format!("github:{o}/{r}/{rev}"));
                }
            }
        }
    }
    inp.url.clone()
}

/// Channel name for an input URL ref, e.g. "nixos-25.11" -> "25_11".
pub fn channel_name_for_ref(r: &str) -> String {
    let r = r.trim().trim_start_matches("nixos-").trim_start_matches("nixpkgs-");
    r.replace('.', "_").replace('-', "_")
}

pub fn nixpkgs_url_for_ref(r: &str) -> String {
    format!("github:nixos/nixpkgs?ref={}", r.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_and_is_recognised() {
        let t = template("box", "alice", "x86_64-linux", DEFAULT_NIXPKGS_REF);
        let i = info_from_source(t.clone(), None, BTreeMap::new());
        assert!(i.parse_errors.is_empty());
        assert!(i.has_template_structure);
        assert_eq!(i.hosts, vec!["box"]);
        assert_eq!(i.inputs[0].name, "nixpkgs");
        assert_eq!(i.inputs[0].url.as_deref(), Some("github:nixos/nixpkgs?ref=nixos-unstable"));
        assert_eq!(i.channels, Some(vec![]));
        assert_eq!(i.vars.as_ref().unwrap()[0].name, "mainUsername");
        assert_eq!(i.system.as_deref(), Some("x86_64-linux"));
    }

    #[test]
    fn add_channel_and_vars_roundtrip() {
        let t = template("box", "alice", "x86_64-linux", DEFAULT_NIXPKGS_REF);
        let s = apply(&t, &FlakeOp::AddChannel { name: "25_11".into(), url: nixpkgs_url_for_ref("nixos-25.11") }).unwrap();
        let i = info_from_source(s.clone(), None, BTreeMap::new());
        assert_eq!(i.channels, Some(vec!["25_11".to_string()]));
        assert!(i.inputs.iter().any(|x| x.name == "nixpkgs-25_11" && x.is_nixpkgs_channel));
        let s = apply(&s, &FlakeOp::SetVar { name: "mainUserEmail".into(), value: "\"a@b.c\"".into() }).unwrap();
        let s = apply(&s, &FlakeOp::SetVar { name: "mainUsername".into(), value: "\"bob\"".into() }).unwrap();
        let i = info_from_source(s.clone(), None, BTreeMap::new());
        let vars = i.vars.unwrap();
        assert!(vars.iter().any(|v| v.name == "mainUserEmail" && v.value == "\"a@b.c\""));
        assert!(vars.iter().any(|v| v.name == "mainUsername" && v.value == "\"bob\""));
        let s = apply(&s, &FlakeOp::RemoveChannel { name: "25_11".into() }).unwrap();
        let s = apply(&s, &FlakeOp::RemoveVar { name: "mainUserEmail".into() }).unwrap();
        let i = info_from_source(s, None, BTreeMap::new());
        assert_eq!(i.channels, Some(vec![]));
        assert!(!i.inputs.iter().any(|x| x.name == "nixpkgs-25_11"));
        assert!(!i.vars.unwrap().iter().any(|v| v.name == "mainUserEmail"));
    }

    #[test]
    fn inputs_with_follows_and_removal() {
        let t = template("box", "alice", "x86_64-linux", DEFAULT_NIXPKGS_REF);
        let s = apply(&t, &FlakeOp::AddInput { name: "home-manager".into(), url: "github:nix-community/home-manager".into(), follows_nixpkgs: true }).unwrap();
        let i = info_from_source(s.clone(), None, BTreeMap::new());
        let hm = i.inputs.iter().find(|x| x.name == "home-manager").unwrap();
        assert_eq!(hm.follows.len(), 1);
        assert!(apply(&s, &FlakeOp::AddInput { name: "home-manager".into(), url: "x".into(), follows_nixpkgs: false }).is_err());
        let s = apply(&s, &FlakeOp::SetInputUrl { name: "nixpkgs".into(), url: "github:nixos/nixpkgs?ref=nixos-26.05".into() }).unwrap();
        let s = apply(&s, &FlakeOp::RemoveInput { name: "home-manager".into() }).unwrap();
        let i = info_from_source(s, None, BTreeMap::new());
        assert!(!i.inputs.iter().any(|x| x.name == "home-manager"));
        assert_eq!(i.inputs[0].url.as_deref(), Some("github:nixos/nixpkgs?ref=nixos-26.05"));
    }

    #[test]
    fn stale_lock_is_ignored() {
        assert!(lock_matches(Some("github:nixos/nixpkgs?ref=nixos-26.05"), Some("github:NixOS/nixpkgs/nixos-26.05")));
        assert!(!lock_matches(Some("github:nixos/nixpkgs?ref=nixos-unstable"), Some("github:NixOS/nixpkgs/nixos-26.05")));
        assert!(!lock_matches(Some("github:nixos/nixpkgs?ref=nixos-unstable"), Some("github:other/nixpkgs/nixos-unstable")));
        assert!(lock_matches(Some("github:nixos/nixpkgs?ref=nixos-unstable"), None));
        let t = template("box", "alice", "x86_64-linux", DEFAULT_NIXPKGS_REF);
        let mut lock = BTreeMap::new();
        lock.insert("nixpkgs".to_string(), LockedInput { rev: Some("deadbeef".into()), date: None, source: Some("github:NixOS/nixpkgs/nixos-26.05".into()) });
        let i = info_from_source(t, Some("/etc/nixos/flake.lock".into()), lock);
        assert!(i.inputs[0].locked.is_none(), "stale lock must not attach");
    }

    #[test]
    fn branch_and_ref_helpers() {
        assert_eq!(branch_of("github:nixos/nixpkgs?ref=nixos-25.11").as_deref(), Some("nixos-25.11"));
        assert_eq!(branch_of("github:nix-community/home-manager/release-26.05").as_deref(), Some("release-26.05"));
        let inp = FlakeInput { name: "nixpkgs".into(), url: Some("github:nixos/nixpkgs?ref=nixos-25.11".into()), follows: vec![], locked: Some(LockedInput { rev: Some("abc".into()), date: None, source: None }), is_nixpkgs_channel: false };
        assert_eq!(flake_ref_for(&inp).as_deref(), Some("github:nixos/nixpkgs/abc"));
    }

    #[test]
    fn parses_vincents_style_flake() {
        let src = r#"{
  description = "A very basic flake";
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-26.05";
    nixpkgs-unstable.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    home-manager = {
      url = "github:nix-community/home-manager/release-26.05";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nixpkgs-25_11.url = "github:nixos/nixpkgs?ref=nixos-25.11";
  };
  outputs = inputs@{ self, nixpkgs, home-manager, ... }:
  let
    vars = { mainUsername = "vincent"; mainNixosVersion = "26.05"; };
    system = "x86_64-linux";
    channels = [ "25_11" "unstable" ];
  in
  {
    nixosConfigurations.main = nixpkgs.lib.nixosSystem { inherit system; modules = [ ./configuration.nix ]; };
  };
}"#;
        let i = info_from_source(src.to_string(), None, BTreeMap::new());
        assert!(i.has_template_structure);
        assert_eq!(i.channels, Some(vec!["25_11".into(), "unstable".into()]));
        assert_eq!(i.inputs.len(), 4);
        let hm = i.inputs.iter().find(|x| x.name == "home-manager").unwrap();
        assert_eq!(hm.url.as_deref(), Some("github:nix-community/home-manager/release-26.05"));
        assert_eq!(hm.follows[0].target, "nixpkgs");
        assert_eq!(i.hosts, vec!["main"]);
        assert_eq!(channel_name_for_ref("nixos-25.11"), "25_11");
    }
}
