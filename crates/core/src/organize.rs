//! "Auto organize": regroup a module's bindings so settings that share an
//! attribute-path prefix live in one nested block, at every depth.
//!
//!   users.users.bob.isNormalUser = true;      users.users.bob = {
//!   users.users.bob.shell = pkgs.zsh;     →     isNormalUser = true;
//!                                               shell = pkgs.zsh;
//!                                             };
//!
//! Rules: two or more bindings under the same prefix become a block; a lone
//! binding keeps its dotted path; a chain with a single branch collapses into
//! a dotted prefix (`fileSystems."/mnt/x" = { ... }`). Existing `{ ... }`
//! values are flattened first so the result is the same whatever the input
//! layout was. Value expressions and comments are kept verbatim. Attribute
//! sets that are `rec`, contain `inherit`, or carry comments that are not
//! attached to an entry are treated as opaque values and left alone.

use crate::nixparse::{attrpath_text, clean_comment, leading_comments, trailing_comment, Parsed};
use crate::{CoreError, Result};
use rnix::ast::{self, HasEntry};
use rnix::{NodeOrToken, SyntaxKind, SyntaxNode};
use rowan::ast::AstNode;

const INDENT: &str = "  ";

/// One leaf binding after flattening.
#[derive(Clone, Debug)]
struct Leaf {
    path: Vec<String>,
    /// Value source, re-indented so continuation lines start at column 0 of
    /// the binding's original indentation.
    value: Vec<String>,
    leading: Vec<String>,
    trailing: Option<String>,
}

/// Something we emit as-is (inherit, or a binding we could not flatten).
#[derive(Clone, Debug)]
enum Item {
    Leaf(Leaf),
    Verbatim { lines: Vec<String>, leading: Vec<String> },
}

pub struct Report {
    pub source: String,
    pub groups_created: usize,
    pub changed: bool,
}

pub fn organize(source: &str) -> Result<Report> {
    let parsed = Parsed::parse(source.to_string());
    if !parsed.errors.is_empty() {
        return Err(CoreError::Parse("file has syntax errors; fix them in code view first".into()));
    }
    let (_, _, body) = parsed.module_body();
    let set = match body {
        Some(ast::Expr::AttrSet(s)) => s,
        Some(ast::Expr::LetIn(li)) => match li.body() {
            Some(ast::Expr::AttrSet(s)) => s,
            _ => return Err(CoreError::Invalid("module body is not an attribute set".into())),
        },
        _ => return Err(CoreError::Invalid("module body is not an attribute set".into())),
    };
    if set.rec_token().is_some() {
        return Err(CoreError::Invalid("the module body is a `rec` set; organize is skipped".into()));
    }
    let range = set.syntax().text_range();
    let start: usize = range.start().into();
    let end: usize = range.end().into();
    let base_indent = line_indent(source, start);

    let items = collect(source, &set, &[]);
    let mut groups = 0;
    let body_text = emit(&items, &format!("{base_indent}{INDENT}"), &mut groups, true);
    let new_set = format!("{{\n{body_text}{base_indent}}}");
    let mut out = String::with_capacity(source.len() + 64);
    out.push_str(&source[..start]);
    out.push_str(&new_set);
    out.push_str(&source[end..]);

    let reparsed = rnix::Root::parse(&out);
    if !reparsed.errors().is_empty() {
        return Err(CoreError::Other("organize produced invalid Nix; nothing was changed".into()));
    }
    let changed = out != source;
    Ok(Report { source: out, groups_created: groups, changed })
}

fn line_indent(source: &str, offset: usize) -> String {
    let ls = source[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0);
    source[ls..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

/// Entries of `set` as leaves (flattening nested plain attrsets), paths
/// prefixed with `prefix`.
fn collect(source: &str, set: &ast::AttrSet, prefix: &[String]) -> Vec<Item> {
    let mut out = Vec::new();
    for entry in set.entries() {
        match entry {
            ast::Entry::Inherit(inh) => {
                let leading = leading_comments(inh.syntax(), false);
                let lines = reindented_lines(source, inh.syntax());
                out.push(Item::Verbatim { lines, leading });
            }
            ast::Entry::AttrpathValue(av) => {
                let Some(p) = av.attrpath() else { continue };
                let mut path = prefix.to_vec();
                path.extend(p.attrs().map(|a| a.syntax().text().to_string()));
                let leading = leading_comments(av.syntax(), false);
                let trailing = trailing_comment(av.syntax());
                match av.value() {
                    Some(ast::Expr::AttrSet(inner)) if flattenable(&inner) && leading_is_only_prose(&leading) => {
                        // Comments attached to the block itself move onto its first child.
                        let mut children = collect(source, &inner, &path);
                        if !leading.is_empty() || trailing.is_some() {
                            if let Some(first) = children.first_mut() {
                                let mut l = leading.clone();
                                if let Some(t) = &trailing {
                                    l.push(t.clone());
                                }
                                match first {
                                    Item::Leaf(leaf) => {
                                        l.extend(leaf.leading.drain(..));
                                        leaf.leading = l;
                                    }
                                    Item::Verbatim { leading: vl, .. } => {
                                        l.extend(vl.drain(..));
                                        *vl = l;
                                    }
                                }
                            }
                        }
                        out.extend(children);
                    }
                    Some(v) => {
                        let value = reindented_lines(source, v.syntax());
                        out.push(Item::Leaf(Leaf { path, value, leading, trailing }));
                    }
                    None => {
                        let lines = reindented_lines(source, av.syntax());
                        out.push(Item::Verbatim { lines, leading });
                    }
                }
            }
        }
    }
    out
}

fn leading_is_only_prose(_l: &[String]) -> bool {
    true
}

/// A plain `{ ... }` whose comments all belong to entries can be flattened.
fn flattenable(set: &ast::AttrSet) -> bool {
    if set.rec_token().is_some() {
        return false;
    }
    let mut entries = 0;
    for e in set.entries() {
        if matches!(e, ast::Entry::Inherit(_)) {
            return false;
        }
        entries += 1;
    }
    if entries == 0 {
        return false; // `{ }` stays a value
    }
    // Every comment token directly inside the set must be attached to an entry.
    let attached: usize = set
        .entries()
        .map(|e| leading_comments(e.syntax(), false).len() + trailing_comment(e.syntax()).map(|_| 1).unwrap_or(0))
        .sum();
    let total = set.syntax().children_with_tokens().filter(|c| matches!(c, NodeOrToken::Token(t) if t.kind() == SyntaxKind::TOKEN_COMMENT)).count();
    attached == total
}

/// Source lines of a node with its original line indentation removed from
/// continuation lines, so they can be re-indented at a new depth.
fn reindented_lines(source: &str, node: &SyntaxNode) -> Vec<String> {
    let r = node.text_range();
    let s: usize = r.start().into();
    let e: usize = r.end().into();
    let base = line_indent(source, s);
    let text = &source[s..e];
    text.lines()
        .enumerate()
        .map(|(i, l)| if i == 0 { l.to_string() } else { l.strip_prefix(base.as_str()).unwrap_or(l.trim_start()).to_string() })
        .collect()
}

/// Group items by path prefix and emit text at `indent`. Blank lines
/// separate multi-line blocks at the top level only.
fn emit(items: &[Item], indent: &str, groups: &mut usize, top: bool) -> String {
    // Work on a list where leaves are grouped by first segment, in order of
    // first appearance; verbatim items are emitted where they occur.
    #[derive(Debug)]
    enum Slot {
        Group(String, Vec<Leaf>),
        Verbatim(Vec<String>, Vec<String>),
    }
    let mut slots: Vec<Slot> = Vec::new();
    for it in items {
        match it {
            Item::Verbatim { lines, leading } => slots.push(Slot::Verbatim(lines.clone(), leading.clone())),
            Item::Leaf(leaf) => {
                let head = leaf.path.first().cloned().unwrap_or_default();
                match slots.iter_mut().find(|s| matches!(s, Slot::Group(h, _) if *h == head)) {
                    Some(Slot::Group(_, v)) => v.push(leaf.clone()),
                    _ => slots.push(Slot::Group(head, vec![leaf.clone()])),
                }
            }
        }
    }

    let mut blocks: Vec<(bool, String)> = Vec::new(); // (is multi-line block, text)
    for slot in slots {
        match slot {
            Slot::Verbatim(lines, leading) => {
                let mut t = String::new();
                for c in &leading {
                    t.push_str(&format!("{indent}# {c}\n"));
                }
                for (i, l) in lines.iter().enumerate() {
                    t.push_str(indent);
                    t.push_str(l);
                    if i + 1 < lines.len() {
                        t.push('\n');
                    }
                }
                t.push('\n');
                blocks.push((lines.len() > 1, t));
            }
            Slot::Group(_, leaves) => blocks.push(emit_group(leaves, indent, groups)),
        }
    }

    let mut out = String::new();
    for (i, (multi, text)) in blocks.iter().enumerate() {
        if top && i > 0 && (*multi || blocks[i - 1].0) {
            out.push('\n');
        }
        out.push_str(text);
    }
    out
}

/// Emit one group of leaves that share `path[0]`.
fn emit_group(mut leaves: Vec<Leaf>, indent: &str, groups: &mut usize) -> (bool, String) {
    if leaves.len() == 1 {
        let leaf = leaves.remove(0);
        return emit_leaf(&leaf, &leaf.path.clone(), indent);
    }
    // Longest common prefix of all paths (at least the first segment).
    let mut common = 1usize;
    'outer: loop {
        let seg = leaves[0].path.get(common).cloned();
        let Some(seg) = seg else { break };
        for l in &leaves {
            if l.path.get(common) != Some(&seg) {
                break 'outer;
            }
        }
        common += 1;
    }
    // A leaf whose whole path is the common prefix cannot be grouped with
    // deeper siblings (`a = v; a.b = w;` would be a Nix error): emit all as-is.
    if leaves.iter().any(|l| l.path.len() <= common) {
        let mut t = String::new();
        for l in &leaves {
            t.push_str(&emit_leaf(l, &l.path, indent).1);
        }
        return (true, t);
    }
    *groups += 1;
    let prefix = leaves[0].path[..common].join(".");
    let inner: Vec<Item> = leaves
        .into_iter()
        .map(|mut l| {
            l.path.drain(..common);
            Item::Leaf(l)
        })
        .collect();
    let inner_indent = format!("{indent}{INDENT}");
    let body = emit(&inner, &inner_indent, groups, false);
    (true, format!("{indent}{prefix} = {{\n{body}{indent}}};\n"))
}

fn emit_leaf(leaf: &Leaf, path: &[String], indent: &str) -> (bool, String) {
    let mut t = String::new();
    for c in &leaf.leading {
        t.push_str(&format!("{indent}# {c}\n"));
    }
    let key = path.join(".");
    t.push_str(&format!("{indent}{key} = "));
    for (i, l) in leaf.value.iter().enumerate() {
        if i > 0 {
            t.push('\n');
            if !l.is_empty() {
                t.push_str(indent);
            }
        }
        t.push_str(l);
    }
    t.push(';');
    if let Some(c) = &leaf.trailing {
        t.push_str(&format!(" # {c}"));
    }
    t.push('\n');
    (leaf.value.len() > 1, t)
}

#[allow(dead_code)]
fn _unused(s: &str) -> String {
    clean_comment(s);
    attrpath_text(&ast::Attrpath::cast(rnix::Root::parse("{ a = 1; }").syntax()).unwrap_or_else(|| unreachable!()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vincents_example() {
        let src = r#"{ vars, config, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ cowsay ];
  test1 = "test2/${vars.mainUsername}";
  fileSystems."/mnt/test1".device = "/dev/disk/by-uuid/awdwadaw";
  fileSystems."/mnt/test1".fsType = "ext4";
  fileSystems."/mnt/test1".options = [ "defaults" "nofail" "users" ];
  users.users.test2.isNormalUser = true;
  users.users.test2.description = "test2";
  users.users.test2.home = "/home/test2";
  users.users.test2.createHome = true;
  users.users.test2.extraGroups = [ "wheel" "networkmanager" ];
  users.users.test2.shell = pkgs.bash;
}
"#;
        let r = organize(src).unwrap();
        let expected = r#"{ vars, config, pkgs, ... }:

{
  environment.systemPackages = with pkgs; [ cowsay ];
  test1 = "test2/${vars.mainUsername}";

  fileSystems."/mnt/test1" = {
    device = "/dev/disk/by-uuid/awdwadaw";
    fsType = "ext4";
    options = [ "defaults" "nofail" "users" ];
  };

  users.users.test2 = {
    isNormalUser = true;
    description = "test2";
    home = "/home/test2";
    createHome = true;
    extraGroups = [ "wheel" "networkmanager" ];
    shell = pkgs.bash;
  };
}
"#;
        assert_eq!(r.source, expected);
        assert_eq!(r.groups_created, 2);
        // Idempotent.
        let again = organize(&r.source).unwrap();
        assert_eq!(again.source, r.source);
        assert!(!again.changed);
    }

    #[test]
    fn multiple_layers_and_existing_blocks() {
        let src = r#"{ config, pkgs, ... }:
{
  # Remote access.
  services.openssh.enable = true;
  services.openssh.settings.PasswordAuthentication = false;
  services.openssh.settings.PermitRootLogin = "no"; # keep root out
  services.printing.enable = false;
  services.pipewire = {
    enable = true;
    alsa.enable = true;
    pulse.enable = true;
  };
  networking.firewall.allowedTCPPorts = [
    22
    80
  ];
}
"#;
        let r = organize(src).unwrap();
        let expected = r#"{ config, pkgs, ... }:
{
  services = {
    openssh = {
      # Remote access.
      enable = true;
      settings = {
        PasswordAuthentication = false;
        PermitRootLogin = "no"; # keep root out
      };
    };
    printing.enable = false;
    pipewire = {
      enable = true;
      alsa.enable = true;
      pulse.enable = true;
    };
  };

  networking.firewall.allowedTCPPorts = [
    22
    80
  ];
}
"#;
        assert_eq!(r.source, expected);
        assert_eq!(organize(&r.source).unwrap().source, r.source);
    }

    #[test]
    fn leaves_unsafe_sets_alone() {
        let src = "{ ... }:\n{\n  a.b = 1;\n  a = rec { c = 2; d = c; };\n  inherit (x) y;\n  imports = [ ./a.nix ];\n}\n";
        let r = organize(src).unwrap();
        assert!(r.source.contains("a.b = 1;"));
        assert!(r.source.contains("a = rec { c = 2; d = c; };"));
        assert!(r.source.contains("inherit (x) y;"));
        assert!(r.source.contains("imports = [ ./a.nix ];"));
        assert!(rnix::Root::parse(&r.source).errors().is_empty());
    }

    #[test]
    fn let_in_body_and_errors() {
        let src = "{ ... }:\nlet x = 1; in\n{\n  a.b = x;\n  a.c = 2;\n}\n";
        let r = organize(src).unwrap();
        assert!(r.source.contains("let x = 1; in\n{\n  a = {\n    b = x;\n    c = 2;\n  };\n}\n"), "{}", r.source);
        assert!(organize("{ broken").is_err());
    }
}
