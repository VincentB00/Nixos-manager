//! Lossless, span-based edits on .nix files. Every operation re-parses the
//! file, locates the target by node id, performs a text splice, re-parses the
//! result and rejects it if the edit introduced parse errors.

use crate::discover::resolve_inside;
use crate::nixparse::{self, analyze_file, parse_node_id, version_of, Parsed};
use crate::types::{EditOp, NixFileView, ScalarValue};
use crate::{CoreError, Result};
use rnix::ast::{self, HasEntry};
use rnix::{SyntaxKind, SyntaxNode, TextRange, TextSize};
use rowan::ast::AstNode;
use std::path::Path;

pub fn apply(root: &Path, rel: &str, expected_version: &str, op: &EditOp) -> Result<NixFileView> {
    let full = resolve_inside(root, rel)?;
    let source = std::fs::read_to_string(&full).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => CoreError::NotFound(format!("{rel} not found")),
        _ => CoreError::Io(e),
    })?;
    if version_of(&source) != expected_version {
        return Err(CoreError::Conflict(format!("{rel} changed on disk since it was loaded; reload and retry")));
    }
    let new_source = if rel.ends_with(".nix") {
        let mut out = apply_to_source(&source, op)?;
        let inserted: Option<String> = match op {
            EditOp::SetScalar { value, .. } => Some(render_scalar(value)),
            EditOp::SetRaw { source: t, .. } | EditOp::ListAppend { source: t, .. } | EditOp::AddBinding { source: t, .. } => Some(t.clone()),
            _ => None,
        };
        if let Some(text) = inserted {
            for arg in module_args_in(&text) {
                out = ensure_module_arg(&out, &arg)?;
            }
        }
        out
    } else {
        // Not a Nix file: only whole-file replacement makes sense, unvalidated.
        match op {
            EditOp::ReplaceFile { source: new } => new.clone(),
            _ => return Err(CoreError::Invalid(format!("{rel} is not a Nix file; only whole-file edits are possible"))),
        }
    };
    write_atomic(&full, &new_source)?;
    Ok(analyze_file(rel, new_source))
}

pub fn create_file(root: &Path, rel: &str, source: &str, import_into: Option<&str>) -> Result<()> {
    let full = resolve_inside(root, rel)?;
    if full.exists() {
        return Err(CoreError::Conflict(format!("{rel} already exists")));
    }
    let parsed = Parsed::parse(source.to_string());
    if !parsed.errors.is_empty() {
        return Err(CoreError::Parse(parsed.errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ")));
    }
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomic(&full, source)?;
    if let Some(target) = import_into {
        let target_full = resolve_inside(root, target)?;
        let target_src = std::fs::read_to_string(&target_full)?;
        let target_dir = Path::new(target).parent().unwrap_or(Path::new(""));
        let relative = pathdiff(Path::new(rel), target_dir);
        let new_src = add_import(&target_src, &relative)?;
        write_atomic(&target_full, &new_src)?;
    }
    Ok(())
}

fn pathdiff(target: &Path, from_dir: &Path) -> String {
    // Both relative to the config root.
    let t: Vec<_> = target.components().collect();
    let f: Vec<_> = from_dir.components().collect();
    let common = t.iter().zip(f.iter()).take_while(|(a, b)| a == b).count();
    let mut out = String::new();
    for _ in common..f.len() {
        out.push_str("../");
    }
    if out.is_empty() {
        out.push_str("./");
    }
    let rest: Vec<String> = t[common..].iter().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    out.push_str(&rest.join("/"));
    out
}

/// Add `path` to the file's `imports = [ ... ]`, creating the list if needed.
pub fn add_import(source: &str, path: &str) -> Result<String> {
    let parsed = Parsed::parse(source.to_string());
    for node in parsed.root.syntax().descendants() {
        if let Some(av) = ast::AttrpathValue::cast(node) {
            let is_imports = av.attrpath().map(|p| nixparse::attrpath_text(&p) == "imports").unwrap_or(false);
            if is_imports {
                if let Some(v) = av.value() {
                    if let Some(list) = find_list(v.syntax()) {
                        return splice_checked(source, list_append_edit(source, &list, path)?);
                    }
                }
            }
        }
    }
    // No imports list: add one to the module body.
    let (_, _, body) = parsed.module_body();
    let Some(ast::Expr::AttrSet(set)) = body else {
        return Err(CoreError::Invalid("cannot find an attribute set to add imports to".into()));
    };
    splice_checked(source, add_binding_edit(source, set.syntax(), "imports", &format!("[ {path} ]"))?)
}

pub fn apply_to_source(source: &str, op: &EditOp) -> Result<String> {
    if let EditOp::ReplaceFile { source: new } = op {
        let p = Parsed::parse(new.clone());
        if !p.errors.is_empty() {
            return Err(CoreError::Parse(p.errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ")));
        }
        return Ok(new.clone());
    }
    let parsed = Parsed::parse(source.to_string());
    let edit = match op {
        EditOp::SetScalar { node_id, value } => {
            let (vrange, _) = locate_value(&parsed, node_id)?;
            Edit { range: vrange, text: render_scalar(value) }
        }
        EditOp::SetRaw { node_id, source: text } => {
            let (vrange, _) = locate_value(&parsed, node_id)?;
            validate_expr(text)?;
            Edit { range: vrange, text: text.clone() }
        }
        EditOp::ListAppend { node_id, source: item } => {
            let (vrange, _) = locate_value(&parsed, node_id)?;
            validate_expr(item)?;
            let node = covering_node(&parsed, vrange)?;
            let list = find_list(&node).ok_or_else(|| CoreError::Invalid("target is not a list".into()))?;
            list_append_edit(source, &list, item)?
        }
        EditOp::ListRemove { node_id } => {
            let (start, end) = parse_node_id(node_id).ok_or_else(|| CoreError::Invalid("bad node id".into()))?;
            remove_item_edit(source, start, end)
        }
        EditOp::ToggleDisabled { node_id } => {
            let (start, end) = parse_node_id(node_id).ok_or_else(|| CoreError::Invalid("bad node id".into()))?;
            toggle_edit(source, start, end)?
        }
        EditOp::AddBinding { node_id, key, source: value } => {
            validate_expr(value)?;
            validate_key(key)?;
            let (vrange, _) = locate_value(&parsed, node_id)?;
            let node = covering_node(&parsed, vrange)?;
            let set = find_attrset(&node).ok_or_else(|| CoreError::Invalid("target is not an attribute set".into()))?;
            add_binding_edit(source, &set, key, value)?
        }
        EditOp::RemoveBinding { node_id } => {
            let (start, end) = parse_node_id(node_id).ok_or_else(|| CoreError::Invalid("bad node id".into()))?;
            remove_item_edit(source, start, end)
        }
        EditOp::EnsureArg { name } => {
            validate_key(name)?;
            return ensure_module_arg(source, name);
        }
        EditOp::ReplaceFile { .. } => unreachable!(),
    };
    splice_checked(source, edit)
}

pub(crate) struct Edit {
    pub(crate) range: TextRange,
    pub(crate) text: String,
}

pub(crate) fn splice_checked(source: &str, edit: Edit) -> Result<String> {
    let s: usize = edit.range.start().into();
    let e: usize = edit.range.end().into();
    if s > e || e > source.len() {
        return Err(CoreError::Invalid("edit range out of bounds".into()));
    }
    let before_errors = rnix::Root::parse(source).errors().len();
    let mut out = String::with_capacity(source.len() + edit.text.len());
    out.push_str(&source[..s]);
    out.push_str(&edit.text);
    out.push_str(&source[e..]);
    let after = rnix::Root::parse(&out);
    if after.errors().len() > before_errors {
        return Err(CoreError::Parse(format!(
            "edit would produce invalid Nix: {}",
            after.errors().iter().map(|e| e.to_string()).collect::<Vec<_>>().join("; ")
        )));
    }
    Ok(out)
}

fn range_of(id: &str) -> Result<TextRange> {
    let (s, e) = parse_node_id(id).ok_or_else(|| CoreError::Invalid(format!("bad node id {id}")))?;
    Ok(TextRange::new(TextSize::from(s as u32), TextSize::from(e as u32)))
}

/// Node ids are "<entry start>-<entry end>". The value range is what the
/// tree reported as `value_span`; we recompute it by rebuilding the tree.
fn locate_value(parsed: &Parsed, node_id: &str) -> Result<(TextRange, TextRange)> {
    let (_, _, body) = parsed.module_body();
    let tree = nixparse::TreeBuilder::new(parsed).build_root(body);
    fn find<'a>(n: &'a crate::types::Node, id: &str) -> Option<&'a crate::types::Node> {
        if n.id == id {
            return Some(n);
        }
        n.children.iter().find_map(|c| find(c, id))
    }
    let node = find(&tree, node_id).ok_or_else(|| CoreError::NotFound(format!("node {node_id} not found; reload the file")))?;
    if node.disabled {
        return Err(CoreError::Invalid("item is commented out; enable it first".into()));
    }
    let v = TextRange::new(node.value_span.start.into(), node.value_span.end.into());
    let e = TextRange::new(node.span.start.into(), node.span.end.into());
    Ok((v, e))
}

fn covering_node(parsed: &Parsed, range: TextRange) -> Result<SyntaxNode> {
    let root = parsed.root.syntax().clone();
    let el = root.covering_element(range);
    let node = match el {
        rnix::NodeOrToken::Node(n) => n,
        rnix::NodeOrToken::Token(t) => t.parent().ok_or_else(|| CoreError::Invalid("no node".into()))?,
    };
    // Walk up until the node exactly matches the range.
    let mut cur = node;
    while cur.text_range() != range {
        match cur.parent() {
            Some(p) if p.text_range().contains_range(range) => cur = p,
            _ => break,
        }
    }
    Ok(cur)
}

/// Descend through `with`/parens to the list.
fn find_list(node: &SyntaxNode) -> Option<ast::List> {
    if let Some(l) = ast::List::cast(node.clone()) {
        return Some(l);
    }
    if let Some(w) = ast::With::cast(node.clone()) {
        return w.body().and_then(|b| find_list(b.syntax()));
    }
    if let Some(p) = ast::Paren::cast(node.clone()) {
        return p.expr().and_then(|e| find_list(e.syntax()));
    }
    None
}

fn find_attrset(node: &SyntaxNode) -> Option<SyntaxNode> {
    if node.kind() == SyntaxKind::NODE_ATTR_SET {
        return Some(node.clone());
    }
    if let Some(p) = ast::Paren::cast(node.clone()) {
        return p.expr().and_then(|e| find_attrset(e.syntax()));
    }
    if let Some(li) = ast::LetIn::cast(node.clone()) {
        return li.body().and_then(|e| find_attrset(e.syntax()));
    }
    if let Some(w) = ast::With::cast(node.clone()) {
        return w.body().and_then(|e| find_attrset(e.syntax()));
    }
    None
}

// ───────────────────────── text helpers ─────────────────────────

fn line_start(source: &str, offset: usize) -> usize {
    source[..offset].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

fn line_end(source: &str, offset: usize) -> usize {
    source[offset..].find('\n').map(|i| offset + i).unwrap_or(source.len())
}

fn indent_at(source: &str, offset: usize) -> String {
    let ls = line_start(source, offset);
    source[ls..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

fn alone_on_line(source: &str, start: usize, end: usize) -> bool {
    let ls = line_start(source, start);
    let le = line_end(source, end);
    let before = &source[ls..start];
    let after = source[end..le].trim_start();
    before.trim().is_empty() && (after.is_empty() || after.starts_with('#') || after == ";" || after.starts_with("; #") || after.starts_with(";#"))
}

pub(crate) fn list_append_edit(source: &str, list: &ast::List, item: &str) -> Result<Edit> {
    let r = list.syntax().text_range();
    let ls: usize = r.start().into();
    let le: usize = r.end().into();
    let inner = &source[ls + 1..le - 1];
    let multiline = inner.contains('\n');
    let items: Vec<_> = list.items().collect();
    let at = |pos: usize| TextRange::new((pos as u32).into(), (pos as u32).into());
    if let Some(last) = items.last() {
        let last_end: usize = last.syntax().text_range().end().into();
        if !multiline {
            return Ok(Edit { range: at(last_end), text: format!(" {item}") });
        }
        let close_line_start = line_start(source, le - 1);
        if last_end >= close_line_start {
            // `]` sits on the same line as the last item.
            return Ok(Edit { range: at(last_end), text: format!(" {item}") });
        }
        // Insert as a new line just above the closing bracket's line, so it
        // lands after any trailing comments or commented-out items.
        let indent = indent_at(source, last.syntax().text_range().start().into());
        return Ok(Edit { range: at(close_line_start - 1), text: format!("\n{indent}{item}") });
    }
    // Empty list.
    let bracket_indent = indent_at(source, ls);
    if multiline {
        Ok(Edit { range: TextRange::new(((ls + 1) as u32).into(), ((le - 1) as u32).into()), text: format!("\n{bracket_indent}  {item}\n{bracket_indent}") })
    } else {
        Ok(Edit { range: TextRange::new(((ls + 1) as u32).into(), ((le - 1) as u32).into()), text: format!(" {item} ") })
    }
}

pub(crate) fn remove_item_edit(source: &str, start: usize, end: usize) -> Edit {
    if alone_on_line(source, start, end) {
        let ls = line_start(source, start);
        let mut le = line_end(source, end);
        if le < source.len() {
            le += 1; // eat the newline
        }
        Edit { range: TextRange::new((ls as u32).into(), (le as u32).into()), text: String::new() }
    } else {
        // Inline: remove the item and one following space if present.
        let mut e = end;
        if source[e..].starts_with(' ') {
            e += 1;
        }
        Edit { range: TextRange::new((start as u32).into(), (e as u32).into()), text: String::new() }
    }
}

fn toggle_edit(source: &str, start: usize, end: usize) -> Result<Edit> {
    let text = &source[start..end];
    if text.starts_with('#') {
        // Uncomment every line of the block: drop '#' (and one space) after the
        // indentation; a bare '#' marker restores an empty line.
        let inner: Vec<String> = text.lines().map(|l| {
            let ws_len = l.len() - l.trim_start().len();
            let (ws, body) = l.split_at(ws_len);
            let rest = crate::nixparse::strip_hash(body);
            if rest.is_empty() { String::new() } else { format!("{ws}{rest}") }
        }).collect();
        let inner = inner.join("\n");
        let trimmed = inner.trim();
        if validate_expr(trimmed).is_err() {
            // a binding ("key = value;") or a group: must parse inside a set
            let wrapped = format!("{{\n{trimmed}\n}}");
            let p = rnix::Root::parse(&wrapped);
            if !p.errors().is_empty() {
                return Err(CoreError::Invalid("the commented-out text is not valid Nix any more; edit the file as code instead".into()));
            }
        }
        return Ok(Edit { range: TextRange::new((start as u32).into(), (end as u32).into()), text: inner.trim_end().to_string() });
    }
    if text.contains('\n') {
        // A group spanning several lines: comment every line, keeping the indentation.
        let ls = line_start(source, start);
        if !source[ls..start].trim().is_empty() {
            return Err(CoreError::Invalid("only settings that start on their own line can be commented out".into()));
        }
        // The marker goes at the group's own indentation on every line, so the
        // inner indentation stays visible after "# " (and round-trips exactly).
        let base = &source[ls..start];
        let mut out = String::new();
        for (i, l) in text.split('\n').enumerate() {
            if i == 0 {
                out.push_str("# ");
                out.push_str(l);
                continue;
            }
            out.push('\n');
            if l.trim().is_empty() {
                out.push_str(base);
                out.push('#');
            } else if let Some(rest) = l.strip_prefix(base) {
                out.push_str(base);
                out.push_str("# ");
                out.push_str(rest);
            } else {
                let ws_len = l.len() - l.trim_start().len();
                let (ws, body) = l.split_at(ws_len);
                out.push_str(ws);
                out.push_str("# ");
                out.push_str(body);
            }
        }
        return Ok(Edit { range: TextRange::new((start as u32).into(), (end as u32).into()), text: out });
    }
    if !alone_on_line(source, start, end) {
        return Err(CoreError::Invalid("only items on their own line can be commented out".into()));
    }
    Ok(Edit { range: TextRange::new((start as u32).into(), (start as u32).into()), text: "# ".to_string() })
}

pub(crate) fn add_binding_edit(source: &str, set: &SyntaxNode, key: &str, value: &str) -> Result<Edit> {
    let r = set.text_range();
    let ls: usize = r.start().into();
    let le: usize = r.end().into();
    let set_ast = ast::AttrSet::cast(set.clone()).ok_or_else(|| CoreError::Invalid("not an attrset".into()))?;
    let mut entries: Vec<SyntaxNode> = set_ast.entries().map(|e| e.syntax().clone()).collect();
    let multiline = source[ls..le].contains('\n');
    if let Some(last) = entries.pop() {
        let last_end: usize = last.text_range().end().into();
        let indent = indent_at(source, last.text_range().start().into());
        if multiline {
            let le_line = line_end(source, last_end);
            return Ok(Edit { range: TextRange::new((le_line as u32).into(), (le_line as u32).into()), text: format!("\n{indent}{key} = {value};") });
        }
        return Ok(Edit { range: TextRange::new((last_end as u32).into(), (last_end as u32).into()), text: format!(" {key} = {value};") });
    }
    let indent = indent_at(source, ls);
    if multiline {
        Ok(Edit { range: TextRange::new(((ls + 1) as u32).into(), ((le - 1) as u32).into()), text: format!("\n{indent}  {key} = {value};\n{indent}") })
    } else {
        Ok(Edit { range: TextRange::new(((ls + 1) as u32).into(), ((le - 1) as u32).into()), text: format!(" {key} = {value}; ") })
    }
}

pub fn render_scalar(v: &ScalarValue) -> String {
    match v {
        ScalarValue::Bool { value } => value.to_string(),
        ScalarValue::Int { value } => value.to_string(),
        ScalarValue::Float { value } => {
            let s = value.to_string();
            if s.contains('.') { s } else { format!("{s}.0") }
        }
        ScalarValue::String { value } => nix_string(value),
        ScalarValue::Path { value } => value.trim().to_string(),
        ScalarValue::Ref { value } => value.trim().to_string(),
        ScalarValue::Null => "null".to_string(),
    }
}

pub fn nix_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '$' if chars.peek() == Some(&'{') => out.push_str("\\$"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

pub(crate) fn validate_expr(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        return Err(CoreError::Invalid("value cannot be empty".into()));
    }
    let p = rnix::Root::parse(text);
    if !p.errors().is_empty() {
        return Err(CoreError::Parse(format!("not a valid Nix expression: {}", p.errors()[0])));
    }
    Ok(())
}

fn validate_key(key: &str) -> Result<()> {
    if key.trim().is_empty() {
        return Err(CoreError::Invalid("key cannot be empty".into()));
    }
    let p = rnix::Root::parse(&format!("{{ {key} = 1; }}"));
    if !p.errors().is_empty() {
        return Err(CoreError::Invalid(format!("'{key}' is not a valid attribute path")));
    }
    Ok(())
}

pub fn write_atomic(path: &Path, content: &str) -> Result<()> {
    let tmp = path.with_extension("nixos-manager.tmp");
    std::fs::write(&tmp, content)?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[allow(dead_code)]
fn _unused(r: &str) -> Result<TextRange> {
    range_of(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NodeKind;

    const SAMPLE: &str = r#"{ config, pkgs, ... }:
{
  imports = [
    ./a.nix
    # ./b.nix
  ];
  services.docker.enable = true;
  networking.hostName = "box"; # name
  environment.systemPackages = with pkgs; [
    git # vcs
    vim
  ];
  empty = [ ];
  nested = { x = 1; };
}
"#;

    fn id_of(label: &str) -> String {
        let v = analyze_file("t.nix", SAMPLE.to_string());
        v.root.children.iter().find(|c| c.label.as_deref() == Some(label)).unwrap().id.clone()
    }

    #[test]
    fn set_bool() {
        let out = apply_to_source(SAMPLE, &EditOp::SetScalar { node_id: id_of("services.docker.enable"), value: ScalarValue::Bool { value: false } }).unwrap();
        assert!(out.contains("services.docker.enable = false;"));
    }

    #[test]
    fn set_string_keeps_comment() {
        let out = apply_to_source(SAMPLE, &EditOp::SetScalar { node_id: id_of("networking.hostName"), value: ScalarValue::String { value: "my \"pc\"".into() } }).unwrap();
        assert!(out.contains(r#"networking.hostName = "my \"pc\""; # name"#));
    }

    #[test]
    fn list_append_multiline_keeps_trailing_comment() {
        let out = apply_to_source(SAMPLE, &EditOp::ListAppend { node_id: id_of("environment.systemPackages"), source: "htop".into() }).unwrap();
        assert!(out.contains("    vim\n    htop\n  ];"), "{out}");
    }

    #[test]
    fn list_append_empty() {
        let out = apply_to_source(SAMPLE, &EditOp::ListAppend { node_id: id_of("empty"), source: "1".into() }).unwrap();
        assert!(out.contains("empty = [ 1 ];"));
    }

    #[test]
    fn toggle_import() {
        let v = analyze_file("t.nix", SAMPLE.to_string());
        let imports = &v.root.children[0];
        let disabled = imports.children.iter().find(|c| c.disabled).unwrap();
        let out = apply_to_source(SAMPLE, &EditOp::ToggleDisabled { node_id: disabled.id.clone() }).unwrap();
        assert!(out.contains("    ./a.nix\n    ./b.nix\n"), "{out}");
        let v2 = analyze_file("t.nix", out.clone());
        let enabled = &v2.root.children[0].children[0];
        let out2 = apply_to_source(&out, &EditOp::ToggleDisabled { node_id: enabled.id.clone() }).unwrap();
        assert!(out2.contains("    # ./a.nix\n    ./b.nix\n"), "{out2}");
    }

    #[test]
    fn comment_out_a_group_and_restore_it() {
        let src = "{ config, pkgs, ... }: {\n  services.printing = {\n    enable = true;\n\n    drivers = [ pkgs.gutenprint ];\n  };\n  users.users.v.isNormalUser = true;\n}\n";
        let v = analyze_file("t.nix", src.to_string());
        let grp = v.root.children.iter().find(|c| c.label.as_deref() == Some("services.printing")).unwrap();
        let out = apply_to_source(src, &EditOp::ToggleDisabled { node_id: grp.id.clone() }).unwrap();
        assert!(out.contains("  # services.printing = {\n  #   enable = true;\n  #\n  #   drivers = [ pkgs.gutenprint ];\n  # };\n"), "{out}");
        let v2 = analyze_file("t.nix", out.clone());
        let dis = v2.root.children.iter().find(|c| c.label.as_deref() == Some("services.printing")).unwrap();
        assert!(dis.disabled && dis.kind == NodeKind::Attrset);
        let back = apply_to_source(&out, &EditOp::ToggleDisabled { node_id: dis.id.clone() }).unwrap();
        assert_eq!(back, src, "round trip must restore the file exactly");
        // a single-line setting too
        let v3 = analyze_file("t.nix", src.to_string());
        let one = v3.root.children.iter().find(|c| c.label.as_deref() == Some("users.users.v.isNormalUser")).unwrap();
        let out3 = apply_to_source(src, &EditOp::ToggleDisabled { node_id: one.id.clone() }).unwrap();
        assert!(out3.contains("  # users.users.v.isNormalUser = true;\n"), "{out3}");
        let v4 = analyze_file("t.nix", out3.clone());
        let d4 = v4.root.children.iter().find(|c| c.label.as_deref() == Some("users.users.v.isNormalUser")).unwrap();
        assert!(d4.disabled && d4.kind == NodeKind::Bool);
        assert_eq!(apply_to_source(&out3, &EditOp::ToggleDisabled { node_id: d4.id.clone() }).unwrap(), src);
    }

    #[test]
    fn remove_item_line() {
        let v = analyze_file("t.nix", SAMPLE.to_string());
        let pk = v.root.children.iter().find(|c| c.label.as_deref() == Some("environment.systemPackages")).unwrap();
        let git = &pk.children[0];
        let out = apply_to_source(SAMPLE, &EditOp::ListRemove { node_id: git.id.clone() }).unwrap();
        assert!(!out.contains("git # vcs"));
        assert!(out.contains("[\n    vim\n  ];"), "{out}");
    }

    #[test]
    fn add_and_remove_binding() {
        let out = apply_to_source(SAMPLE, &EditOp::AddBinding { node_id: id_of("nested"), key: "y".into(), source: "2".into() }).unwrap();
        assert!(out.contains("nested = { x = 1; y = 2; };"), "{out}");
        let v = analyze_file("t.nix", SAMPLE.to_string());
        let out = apply_to_source(SAMPLE, &EditOp::AddBinding { node_id: v.root.id.clone(), key: "time.timeZone".into(), source: "\"UTC\"".into() }).unwrap();
        assert!(out.contains("  nested = { x = 1; };\n  time.timeZone = \"UTC\";\n}"), "{out}");
        let out = apply_to_source(SAMPLE, &EditOp::RemoveBinding { node_id: id_of("services.docker.enable") }).unwrap();
        assert!(!out.contains("docker"));
    }

    #[test]
    fn rejects_invalid() {
        assert!(apply_to_source(SAMPLE, &EditOp::SetRaw { node_id: id_of("nested"), source: "{ broken".into() }).is_err());
        assert!(apply_to_source(SAMPLE, &EditOp::AddBinding { node_id: id_of("nested"), key: "bad key".into(), source: "1".into() }).is_err());
    }

    #[test]
    fn add_import_to_existing_list() {
        let out = add_import(SAMPLE, "./c.nix").unwrap();
        assert!(out.contains("    # ./b.nix\n    ./c.nix\n  ];"), "{out}");
        let v = analyze_file("t.nix", out);
        assert_eq!(v.root.children[0].kind, NodeKind::List);
    }
}

// ───────────────────────── wizard: merge-in changes ─────────────────────────

use crate::types::{ConfigChange, Node, NodeKind};

/// Split an attribute path on dots, keeping quoted segments intact.
fn split_attrpath(key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quote = false;
    for c in key.chars() {
        match c {
            '"' => {
                in_quote = !in_quote;
                cur.push(c);
            }
            '.' if !in_quote => {
                out.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The attrset node that holds a module's top-level bindings.
fn body_attrset(root: &Node) -> Option<&Node> {
    match root.kind {
        NodeKind::Attrset => Some(root),
        NodeKind::Letin => root.children.iter().find(|c| c.label.as_deref() == Some("in")).and_then(body_attrset),
        _ => None,
    }
}

enum Found<'a> {
    /// Exact binding exists.
    Exact(&'a Node),
    /// A parent attrset exists; `rest` is the remaining path to add inside it.
    Partial(&'a Node, Vec<String>),
}

fn find_path<'a>(set: &'a Node, segs: &[String]) -> Found<'a> {
    for child in &set.children {
        let Some(label) = &child.label else { continue };
        let parts = split_attrpath(label);
        if parts.len() > segs.len() {
            continue;
        }
        if parts[..] == segs[..parts.len()] {
            if parts.len() == segs.len() {
                return Found::Exact(child);
            }
            if child.kind == NodeKind::Attrset {
                return find_path(child, &segs[parts.len()..]);
            }
        }
    }
    Found::Partial(set, segs.to_vec())
}

fn analyze(source: &str) -> Result<Node> {
    let parsed = Parsed::parse(source.to_string());
    if !parsed.errors.is_empty() {
        return Err(CoreError::Parse("file has syntax errors; fix them in code view first".into()));
    }
    let (_, _, body) = parsed.module_body();
    Ok(nixparse::TreeBuilder::new(&parsed).build_root(body))
}

/// `key = value;`: replace the value if the binding exists, else add it in the
/// deepest existing parent attrset.
pub fn upsert_setting(source: &str, key: &str, value: &str) -> Result<String> {
    validate_expr(value)?;
    validate_key(key)?;
    let root = analyze(source)?;
    let body = body_attrset(&root).ok_or_else(|| CoreError::Invalid("module body is not an attribute set".into()))?;
    let segs = split_attrpath(key);
    match find_path(body, &segs) {
        Found::Exact(node) => {
            if node.disabled {
                return Err(CoreError::Invalid(format!("{key} exists but is commented out")));
            }
            apply_to_source(source, &EditOp::SetRaw { node_id: node.id.clone(), source: value.to_string() })
        }
        Found::Partial(set, rest) => apply_to_source(source, &EditOp::AddBinding { node_id: set.id.clone(), key: rest.join("."), source: value.to_string() }),
    }
}

/// Append `item` to the list option at `key`, creating `key = [ item ];` when
/// absent. Skips items already present. Honors `with pkgs;` lists.
pub fn append_to_list_setting(source: &str, key: &str, item: &str) -> Result<String> {
    validate_expr(item)?;
    validate_key(key)?;
    let root = analyze(source)?;
    let body = body_attrset(&root).ok_or_else(|| CoreError::Invalid("module body is not an attribute set".into()))?;
    let segs = split_attrpath(key);
    match find_path(body, &segs) {
        Found::Exact(node) => {
            if node.kind != NodeKind::List {
                return Err(CoreError::Conflict(format!("{key} exists but is not a plain list; edit it by hand")));
            }
            let mut item_text = item.trim().to_string();
            if let Some(ns) = &node.with_namespace {
                if let Some(stripped) = item_text.strip_prefix(&format!("{ns}.")) {
                    item_text = stripped.to_string();
                }
            }
            let exists = node.children.iter().any(|c| !c.disabled && (c.value.as_deref() == Some(item_text.as_str()) || c.source.as_deref() == Some(item_text.as_str())));
            if exists {
                return Ok(source.to_string());
            }
            apply_to_source(source, &EditOp::ListAppend { node_id: node.id.clone(), source: item_text })
        }
        Found::Partial(set, rest) => apply_to_source(source, &EditOp::AddBinding { node_id: set.id.clone(), key: rest.join("."), source: format!("[ {} ]", item.trim()) }),
    }
}

/// Add a package attr (e.g. "htop" or "python3Packages.pip") from package
/// set `set` (default "pkgs") to environment.systemPackages, creating
/// `with pkgs; [ ... ]` when absent.
pub fn add_package(source: &str, attr: &str, set: Option<&str>) -> Result<String> {
    let set = set.map(|s| s.trim()).filter(|s| !s.is_empty()).unwrap_or("pkgs");
    let attr = attr.trim();
    let attr = attr.strip_prefix(&format!("{set}.")).unwrap_or(attr);
    let attr = if set == "pkgs" { attr.trim_start_matches("pkgs.") } else { attr };
    validate_expr(attr)?;
    let root = analyze(source)?;
    let body = body_attrset(&root).ok_or_else(|| CoreError::Invalid("module body is not an attribute set".into()))?;
    let segs = split_attrpath("environment.systemPackages");
    let item_for = |list: &Node| -> String {
        if list.with_namespace.as_deref() == Some(set) { attr.to_string() } else { format!("{set}.{attr}") }
    };
    match find_path(body, &segs) {
        Found::Exact(node) if node.kind == NodeKind::List => {
            let item = item_for(node);
            let exists = node.children.iter().any(|c| !c.disabled && c.value.as_deref() == Some(item.as_str()));
            if exists {
                return Ok(source.to_string());
            }
            apply_to_source(source, &EditOp::ListAppend { node_id: node.id.clone(), source: item })
        }
        Found::Exact(node) if node.kind == NodeKind::Concat => {
            // `[ a ] ++ [ b ]`: append to the first plain list we can find.
            let target = node.children.iter().find(|c| c.kind == NodeKind::List).ok_or_else(|| CoreError::Conflict("environment.systemPackages is too complex to edit automatically".into()))?;
            let item = item_for(target);
            apply_to_source(source, &EditOp::ListAppend { node_id: target.id.clone(), source: item })
        }
        Found::Exact(_) => Err(CoreError::Conflict("environment.systemPackages exists but is not a list; add the package by hand".into())),
        Found::Partial(parent, rest) => {
            let value = if set == "pkgs" { format!("with pkgs; [ {attr} ]") } else { format!("[ {set}.{attr} ]") };
            apply_to_source(source, &EditOp::AddBinding { node_id: parent.id.clone(), key: rest.join("."), source: value })
        }
    }
}

/// Module arguments `text` relies on: package sets plus `vars` (flake globals).
pub fn module_args_in(text: &str) -> Vec<String> {
    let mut out = package_sets_in(text);
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(pos) = text[i..].find("vars.") {
        let start = i + pos;
        let before_ok = start == 0 || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_' || bytes[start - 1] == b'-' || bytes[start - 1] == b'.');
        if before_ok {
            if !out.iter().any(|o| o == "vars") {
                out.push("vars".into());
            }
            break;
        }
        i = start + 5;
    }
    out
}

/// Package-set identifiers (`pkgs`, `pkgs-unstable`, `pkgs-25_11-32`) used in `text`.
pub fn package_sets_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(pos) = text[i..].find("pkgs") {
        let start = i + pos;
        let before_ok = start == 0 || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_' || bytes[start - 1] == b'-' || bytes[start - 1] == b'.');
        let mut end = start + 4;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'-') {
            end += 1;
        }
        let ident = &text[start..end];
        if before_ok && (ident == "pkgs" || ident.starts_with("pkgs-")) && !out.iter().any(|o| o == ident) {
            out.push(ident.to_string());
        }
        i = end.max(start + 1);
    }
    out
}

/// Make sure the module function receives `arg` (e.g. `pkgs`). No-op when the
/// file is not a function or already has it.
pub fn ensure_module_arg(source: &str, arg: &str) -> Result<String> {
    let parsed = Parsed::parse(source.to_string());
    let Some(ast::Expr::Lambda(l)) = parsed.root.expr() else { return Ok(source.to_string()) };
    let Some(ast::Param::Pattern(p)) = l.param() else { return Ok(source.to_string()) };
    let has = p.pat_entries().any(|e| e.ident().map(|i| i.syntax().text().to_string() == arg).unwrap_or(false));
    if has {
        return Ok(source.to_string());
    }
    let r = p.syntax().text_range();
    let start: usize = r.start().into();
    // Insert right after the opening brace.
    let insert_at = start + 1;
    let text = format!(" {arg},");
    splice_checked(source, Edit { range: TextRange::new((insert_at as u32).into(), (insert_at as u32).into()), text })
}

/// Apply wizard changes in order. Each step re-parses the file.
pub fn apply_changes(source: &str, changes: &[ConfigChange]) -> Result<String> {
    let mut src = source.to_string();
    let mut sets: Vec<String> = Vec::new();
    let mut note = |text: &str| {
        for s in module_args_in(text) {
            if !sets.contains(&s) {
                sets.push(s);
            }
        }
    };
    for c in changes {
        src = match c {
            ConfigChange::Set { key, value } => {
                note(value);
                upsert_setting(&src, key, value)?
            }
            ConfigChange::AppendList { key, item } => {
                note(item);
                append_to_list_setting(&src, key, item)?
            }
            ConfigChange::AddPackage { attr, set } => {
                let set_name = set.clone().unwrap_or_else(|| "pkgs".into());
                note(&format!("{set_name}.x"));
                add_package(&src, attr, set.as_deref())?
            }
        };
    }
    for s in sets {
        src = ensure_module_arg(&src, &s)?;
    }
    Ok(src)
}

#[cfg(test)]
mod wizard_tests {
    use super::*;

    const MODULE: &str = r#"{ config, pkgs, ... }:

{
  services.openssh = {
    enable = false;
    settings.PermitRootLogin = "no";
  };
  environment.systemPackages = with pkgs; [
    git
  ];
  users.users.vincent.extraGroups = [ "wheel" ];
}
"#;

    #[test]
    fn upsert_replaces_nested_and_adds_missing() {
        let out = upsert_setting(MODULE, "services.openssh.enable", "true").unwrap();
        assert!(out.contains("    enable = true;"), "{out}");
        let out = upsert_setting(&out, "services.openssh.settings.PasswordAuthentication", "false").unwrap();
        assert!(out.contains("    settings.PermitRootLogin = \"no\";\n    settings.PasswordAuthentication = false;\n  };"), "{out}");
        let out = upsert_setting(&out, "time.timeZone", "\"UTC\"").unwrap();
        assert!(out.contains("  users.users.vincent.extraGroups = [ \"wheel\" ];\n  time.timeZone = \"UTC\";\n}"), "{out}");
    }

    #[test]
    fn append_handles_with_pkgs_and_dedup() {
        let out = add_package(MODULE, "pkgs.htop", None).unwrap();
        assert!(out.contains("    git\n    htop\n  ];"), "{out}");
        let again = add_package(&out, "htop", Some("pkgs")).unwrap();
        assert_eq!(again, out);
        let out = add_package(&out, "steam", Some("pkgs-unstable")).unwrap();
        assert!(out.contains("    htop\n    pkgs-unstable.steam\n  ];"), "{out}");
        let again = add_package(&out, "pkgs-unstable.steam", Some("pkgs-unstable")).unwrap();
        assert_eq!(again, out);
        let out = append_to_list_setting(&out, "users.users.vincent.extraGroups", "\"docker\"").unwrap();
        assert!(out.contains("[ \"wheel\" \"docker\" ]"), "{out}");
        let out = append_to_list_setting(&out, "fonts.packages", "pkgs.noto-fonts").unwrap();
        assert!(out.contains("fonts.packages = [ pkgs.noto-fonts ];"), "{out}");
    }

    #[test]
    fn creates_package_list_and_pkgs_arg() {
        let src = "{ config, ... }:\n{\n  time.timeZone = \"UTC\";\n}\n";
        let out = apply_changes(src, &[ConfigChange::AddPackage { attr: "htop".into(), set: None }]).unwrap();
        assert!(out.starts_with("{ pkgs, config, ... }:"), "{out}");
        assert!(out.contains("environment.systemPackages = with pkgs; [ htop ];"), "{out}");
        let out = apply_changes(src, &[ConfigChange::AddPackage { attr: "htop".into(), set: Some("pkgs-25_11".into()) }]).unwrap();
        assert!(out.starts_with("{ pkgs-25_11, config, ... }:"), "{out}");
        assert!(out.contains("environment.systemPackages = [ pkgs-25_11.htop ];"), "{out}");
        assert_eq!(package_sets_in("with pkgs; [ pkgs-unstable.a pkgs-25_11-32.b foo.pkgs-x ]"), vec!["pkgs", "pkgs-unstable", "pkgs-25_11-32"]);
        assert_eq!(module_args_in("\"/home/${vars.mainUsername}\""), vec!["vars"]);
        assert_eq!(module_args_in("config.myvars.x"), Vec::<String>::new());
        let out = apply_changes("{ config, ... }:\n{\n}\n", &[ConfigChange::Set { key: "networking.hostName".into(), value: "vars.hostName".into() }]).unwrap();
        assert!(out.starts_with("{ vars, config, ... }:"), "{out}");
    }

    #[test]
    fn empty_module_gets_bindings() {
        let src = "{ config, pkgs, ... }:\n\n{\n  # Put options here\n}\n";
        let out = apply_changes(src, &[ConfigChange::Set { key: "services.openssh.enable".into(), value: "true".into() }, ConfigChange::AppendList { key: "networking.firewall.allowedTCPPorts".into(), item: "80".into() }]).unwrap();
        assert!(out.contains("services.openssh.enable = true;"), "{out}");
        assert!(out.contains("networking.firewall.allowedTCPPorts = [ 80 ];"), "{out}");
        assert!(Parsed::parse(out.clone()).errors.is_empty());
    }

    #[test]
    fn rejects_broken_file() {
        assert!(upsert_setting("{ broken", "a.b", "1").is_err());
    }
}
