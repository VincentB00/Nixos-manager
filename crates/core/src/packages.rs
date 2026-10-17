//! Package search through the system's `nix search nixpkgs`. The first query
//! builds nix's evaluation cache (around 20 s); later queries take well under
//! a second. Results are also memoised per query in memory.

use crate::process;
use crate::types::{PackageInfo, PackageSearchResult};
use crate::{CoreError, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

const MAX_RESULTS: usize = 60;

#[derive(Clone, Default)]
pub struct PackageSearch {
    cache: Arc<Mutex<HashMap<String, Vec<PackageInfo>>>>,
    warmed: Arc<Mutex<std::collections::HashSet<String>>>,
}

impl PackageSearch {
    /// Search `installable` (a flake ref such as "github:nixos/nixpkgs/<rev>",
    /// default the registry's "nixpkgs"); `set` is echoed back for the UI.
    pub async fn search(&self, query: &str, installable: Option<&str>, set: &str) -> Result<PackageSearchResult> {
        let q = query.trim().to_lowercase();
        let inst = installable.filter(|s| !s.trim().is_empty()).unwrap_or("nixpkgs").to_string();
        let key = format!("{inst}\u{0}{q}");
        if q.len() < 2 {
            return Ok(PackageSearchResult { query: q, set: set.into(), results: vec![], truncated: false, slow: false });
        }
        if let Some(hit) = self.cache.lock().await.get(&key) {
            return Ok(PackageSearchResult { query: q.clone(), set: set.into(), truncated: hit.len() >= MAX_RESULTS, results: hit.clone(), slow: false });
        }
        let slow = !self.warmed.lock().await.contains(&inst);
        // Each word is a separate regex; nix ANDs them.
        let mut args: Vec<String> = vec!["search".into(), inst.clone(), "--json".into(), "--no-write-lock-file".into()];
        for w in q.split_whitespace() {
            args.push(regex_escape(w));
        }
        let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let out = process::run("nix", &args_ref, None, Duration::from_secs(300)).await?;
        self.warmed.lock().await.insert(inst.clone());
        if out.status != 0 {
            if out.stderr.contains("no results") {
                return Ok(PackageSearchResult { query: q, set: set.into(), results: vec![], truncated: false, slow });
            }
            return Err(CoreError::Other(format!("nix search failed: {}", out.stderr.trim())));
        }
        let raw: HashMap<String, RawPkg> = serde_json::from_str(&out.stdout).unwrap_or_default();
        let mut results: Vec<PackageInfo> = raw
            .into_iter()
            .map(|(full, p)| PackageInfo {
                attr: strip_prefix(&full),
                pname: p.pname.unwrap_or_default(),
                version: p.version.unwrap_or_default(),
                description: p.description.unwrap_or_default(),
            })
            .collect();
        results.sort_by(|a, b| rank(a, &q).cmp(&rank(b, &q)).then_with(|| a.attr.cmp(&b.attr)));
        let truncated = results.len() > MAX_RESULTS;
        results.truncate(MAX_RESULTS);
        self.cache.lock().await.insert(key, results.clone());
        Ok(PackageSearchResult { query: q, set: set.into(), results, truncated, slow })
    }
}

/// Lower is better. Exact attribute, then attribute prefix, then a whole word
/// in the attribute or description; well-known desktop programs get a boost,
/// libraries / language-set members / -bin variants are demoted unless the
/// query asks for them.
fn rank(p: &PackageInfo, q: &str) -> i32 {
    let a = p.attr.to_lowercase();
    let pn = p.pname.to_lowercase();
    let d = p.description.to_lowercase();
    let word_in = |hay: &str| hay.split(|c: char| !c.is_alphanumeric()).any(|w| w == q);
    let mut r = if a == q || pn == q {
        0
    } else if a.starts_with(q) || pn.starts_with(q) {
        10
    } else if word_in(&a) || word_in(&pn) {
        20
    } else if word_in(&d) {
        30
    } else if a.contains(q) {
        40
    } else {
        50
    };
    if POPULAR.iter().any(|(attr, ..)| *attr == a) {
        r -= 5;
    }
    if a.contains('.') && !q.contains('.') {
        r += 15; // python3Packages.foo etc.
    }
    if (a.starts_with("lib") && !q.starts_with("lib")) || a.ends_with("-bin") || a.ends_with("-unwrapped") || a.contains("-unstable") {
        r += 8;
    }
    r
}

/// Curated starting points for people who do not know attribute names yet.
pub const POPULAR: &[(&str, &str, &str, &str)] = &[
    ("firefox", "Firefox", "Web browser", "Internet"),
    ("chromium", "Chromium", "Web browser (Chrome without Google services)", "Internet"),
    ("thunderbird", "Thunderbird", "E-mail client", "Internet"),
    ("libreoffice", "LibreOffice", "Office suite: documents, spreadsheets, presentations", "Office"),
    ("vlc", "VLC", "Media player for almost any audio and video", "Media"),
    ("spotify", "Spotify", "Music streaming (unfree)", "Media"),
    ("gimp", "GIMP", "Image editor", "Graphics"),
    ("inkscape", "Inkscape", "Vector graphics editor", "Graphics"),
    ("obs-studio", "OBS Studio", "Screen recording and streaming", "Media"),
    ("vscode", "Visual Studio Code", "Code editor (unfree)", "Development"),
    ("vscodium", "VSCodium", "Visual Studio Code without telemetry", "Development"),
    ("git", "Git", "Version control", "Development"),
    ("discord", "Discord", "Chat (unfree)", "Internet"),
    ("signal-desktop", "Signal", "Private messaging", "Internet"),
    ("steam", "Steam", "Games (unfree; better via the Steam recipe)", "Games"),
    ("kdePackages.kate", "Kate", "Text editor (KDE)", "Office"),
    ("htop", "htop", "Process viewer for the terminal", "System"),
    ("wget", "wget", "Download files from the terminal", "System"),
    ("unzip", "unzip", "Extract .zip archives", "System"),
    ("ripgrep", "ripgrep", "Fast search in files (rg)", "Development"),
];

pub fn popular() -> Vec<crate::types::PopularPackage> {
    POPULAR.iter().map(|(attr, title, description, category)| crate::types::PopularPackage { attr: attr.to_string(), title: title.to_string(), description: description.to_string(), category: category.to_string() }).collect()
}

fn strip_prefix(full: &str) -> String {
    // legacyPackages.x86_64-linux.foo.bar → foo.bar
    let mut parts = full.splitn(3, '.');
    match (parts.next(), parts.next(), parts.next()) {
        (Some("legacyPackages"), Some(_), Some(rest)) | (Some("packages"), Some(_), Some(rest)) => rest.to_string(),
        _ => full.to_string(),
    }
}

fn regex_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if "\\^$.|?*+()[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[derive(serde::Deserialize, Default)]
struct RawPkg {
    pname: Option<String>,
    version: Option<String>,
    description: Option<String>,
}
