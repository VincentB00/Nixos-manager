//! Find files in the working folder by name and by content (case-insensitive
//! substring; no regex, no index: configurations are small).

use crate::types::{SearchFile, SearchMatch, SearchResult};
use std::path::Path;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MATCHES_PER_FILE: usize = 50;
const MAX_TOTAL_MATCHES: usize = 400;
const SNIPPET_MAX: usize = 200;

/// `mode`: "name", "content" or anything else for both.
pub fn search(workdir: &Path, query: &str, mode: &str) -> SearchResult {
    let q = query.trim().to_lowercase();
    let mut out = SearchResult { query: query.trim().to_string(), files: vec![], truncated: false };
    if q.is_empty() {
        return out;
    }
    let want_name = mode != "content";
    let want_content = mode != "name";
    let mut total = 0usize;
    for (rel, size) in crate::discover::list_files(workdir) {
        let name_match = want_name && rel.to_lowercase().contains(&q);
        let mut matches = Vec::new();
        if want_content && size <= MAX_FILE_BYTES && rel != "secrets.age" {
            if let Ok(text) = std::fs::read_to_string(workdir.join(&rel)) {
                for (i, line) in text.lines().enumerate() {
                    if line.to_lowercase().contains(&q) {
                        if matches.len() >= MAX_MATCHES_PER_FILE || total >= MAX_TOTAL_MATCHES {
                            out.truncated = true;
                            break;
                        }
                        let t = line.trim();
                        let snippet: String = if t.chars().count() > SNIPPET_MAX { t.chars().take(SNIPPET_MAX).collect::<String>() + "…" } else { t.to_string() };
                        matches.push(SearchMatch { line: (i + 1) as u32, text: snippet });
                        total += 1;
                    }
                }
            }
        }
        if name_match || !matches.is_empty() {
            out.files.push(SearchFile { rel_path: rel, name_match, matches });
        }
    }
    // name matches first, then by number of hits
    out.files.sort_by(|a, b| b.name_match.cmp(&a.name_match).then(b.matches.len().cmp(&a.matches.len())).then(a.rel_path.cmp(&b.rel_path)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_by_name_and_content() {
        let d = std::env::temp_dir().join(format!("nm-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("modules")).unwrap();
        std::fs::write(d.join("configuration.nix"), "{ services.openssh.enable = true;\n  networking.hostName = \"desk\";\n}\n").unwrap();
        std::fs::write(d.join("modules/printing.nix"), "{ services.printing.enable = true; }\n# OpenSSH is elsewhere\n").unwrap();
        std::fs::write(d.join("README.md"), "notes about openssh\n").unwrap();
        let r = search(&d, "OpenSSH", "both");
        let names: Vec<&str> = r.files.iter().map(|f| f.rel_path.as_str()).collect();
        assert_eq!(names.len(), 3, "{names:?}");
        let cfg = r.files.iter().find(|f| f.rel_path == "configuration.nix").unwrap();
        assert_eq!(cfg.matches.len(), 1);
        assert_eq!(cfg.matches[0].line, 1);
        assert!(!cfg.name_match);
        let r = search(&d, "print", "name");
        assert_eq!(r.files.len(), 1);
        assert!(r.files[0].name_match && r.files[0].matches.is_empty());
        let r = search(&d, "print", "content");
        assert_eq!(r.files.len(), 1);
        assert_eq!(r.files[0].matches[0].line, 1);
        assert!(search(&d, "   ", "both").files.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
