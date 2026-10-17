//! Analysis of a single .nix file into the generic [`Node`] tree the UI
//! renders, plus import/reference extraction. Built on rnix's lossless CST so
//! every node carries exact byte spans for later edits.

use crate::types::{ImportRef, ImportVia, NixFileView, Node, NodeKind, ParseErrorInfo, Span};
use rnix::ast::{self, HasEntry};
use rnix::{NodeOrToken, SyntaxKind, SyntaxNode, SyntaxToken, TextRange};
use rowan::ast::AstNode;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

pub struct Parsed {
    pub source: String,
    pub root: ast::Root,
    pub errors: Vec<rnix::ParseError>,
    line_starts: Vec<usize>,
}

impl Parsed {
    pub fn parse(source: String) -> Parsed {
        let parse = rnix::Root::parse(&source);
        let errors = parse.errors().to_vec();
        let root = parse.tree();
        let mut line_starts = vec![0usize];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Parsed { source, root, errors, line_starts }
    }

    pub fn line_of(&self, offset: usize) -> u32 {
        match self.line_starts.binary_search(&offset) {
            Ok(i) => i as u32 + 1,
            Err(i) => i as u32,
        }
    }

    pub fn span(&self, range: TextRange) -> Span {
        let start: usize = range.start().into();
        let end: usize = range.end().into();
        Span { start: start as u32, end: end as u32, line: self.line_of(start), end_line: self.line_of(end.saturating_sub(1).max(start)) }
    }

    pub fn text(&self, range: TextRange) -> &str {
        let s: usize = range.start().into();
        let e: usize = range.end().into();
        &self.source[s..e]
    }

    pub fn version(&self) -> String {
        version_of(&self.source)
    }

    /// The expression after peeling off a module lambda: (args, has_ellipsis, body).
    pub fn module_body(&self) -> (Vec<String>, bool, Option<ast::Expr>) {
        let Some(expr) = self.root.expr() else { return (vec![], false, None) };
        if let ast::Expr::Lambda(l) = &expr {
            let mut args = Vec::new();
            let mut ellipsis = false;
            match l.param() {
                Some(ast::Param::Pattern(p)) => {
                    for e in p.pat_entries() {
                        if let Some(id) = e.ident() {
                            args.push(id.syntax().text().to_string());
                        }
                    }
                    ellipsis = p.ellipsis_token().is_some();
                    if let Some(b) = p.pat_bind() {
                        if let Some(id) = b.ident() {
                            args.push(format!("{}@", id.syntax().text()));
                        }
                    }
                }
                Some(ast::Param::IdentParam(p)) => {
                    if let Some(id) = p.ident() {
                        args.push(id.syntax().text().to_string());
                    }
                }
                None => {}
            }
            return (args, ellipsis, l.body());
        }
        (vec![], false, Some(expr))
    }

    pub fn parse_errors(&self) -> Vec<ParseErrorInfo> {
        self.errors
            .iter()
            .map(|e| {
                let range = match e {
                    rnix::ParseError::Unexpected(r)
                    | rnix::ParseError::UnexpectedExtra(r)
                    | rnix::ParseError::UnexpectedDoubleBind(r)
                    | rnix::ParseError::DuplicatedArgs(r, _) => Some(*r),
                    rnix::ParseError::UnexpectedWanted(_, r, _) => Some(*r),
                    _ => None,
                };
                ParseErrorInfo { message: e.to_string(), span: range.map(|r| self.span(r)) }
            })
            .collect()
    }
}

pub fn version_of(source: &str) -> String {
    let digest = Sha256::digest(source.as_bytes());
    hex::encode(&digest[..8])
}

pub fn node_id(range: TextRange) -> String {
    let s: usize = range.start().into();
    let e: usize = range.end().into();
    format!("{s}-{e}")
}

pub fn parse_node_id(id: &str) -> Option<(usize, usize)> {
    let (a, b) = id.split_once('-')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

// ───────────────────────── comments ─────────────────────────

pub fn clean_comment(raw: &str) -> String {
    let t = raw.trim();
    if let Some(s) = t.strip_prefix('#') {
        return s.trim().to_string();
    }
    if let Some(s) = t.strip_prefix("/*").and_then(|s| s.strip_suffix("*/")) {
        return s.trim().to_string();
    }
    t.to_string()
}

/// Comments on the lines directly above `node`, in order. Stops at a blank line.
pub fn leading_comments(node: &SyntaxNode, skip_code_like: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut el = node.prev_sibling_or_token();
    while let Some(e) = el {
        match &e {
            NodeOrToken::Token(t) => match t.kind() {
                SyntaxKind::TOKEN_WHITESPACE => {
                    if t.text().matches('\n').count() >= 2 {
                        break;
                    }
                }
                SyntaxKind::TOKEN_COMMENT => {
                    if !starts_own_line(t) {
                        // A trailing comment of the previous element, not ours.
                        break;
                    }
                    let c = clean_comment(t.text());
                    if skip_code_like && looks_like_simple_expr(&c) {
                        // a commented-out item, not prose
                    } else {
                        out.push(c);
                    }
                }
                _ => break,
            },
            NodeOrToken::Node(_) => break,
        }
        el = e.prev_sibling_or_token();
    }
    out.reverse();
    out
}

/// True when nothing but whitespace-with-a-newline (or an opening bracket /
/// start of file) precedes this token on its line.
fn starts_own_line(token: &SyntaxToken) -> bool {
    match token.prev_sibling_or_token() {
        None => true,
        Some(NodeOrToken::Token(p)) => match p.kind() {
            SyntaxKind::TOKEN_WHITESPACE => p.text().contains('\n'),
            SyntaxKind::TOKEN_L_BRACE | SyntaxKind::TOKEN_L_BRACK => true,
            _ => false,
        },
        Some(NodeOrToken::Node(_)) => false,
    }
}

/// A `# comment` on the same line after `node`.
pub fn trailing_comment(node: &SyntaxNode) -> Option<String> {
    let mut el = node.next_sibling_or_token();
    while let Some(e) = el {
        match &e {
            NodeOrToken::Token(t) => match t.kind() {
                SyntaxKind::TOKEN_WHITESPACE => {
                    if t.text().contains('\n') {
                        return None;
                    }
                }
                SyntaxKind::TOKEN_COMMENT => return Some(clean_comment(t.text())),
                SyntaxKind::TOKEN_SEMICOLON | SyntaxKind::TOKEN_COMMA => {}
                _ => return None,
            },
            NodeOrToken::Node(_) => return None,
        }
        el = e.next_sibling_or_token();
    }
    None
}

fn join_comments(leading: Vec<String>, trailing: Option<String>) -> Option<String> {
    let mut parts = leading;
    if let Some(t) = trailing {
        parts.push(t);
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

/// Heuristic: does a comment's text look like a single commented-out Nix
/// value (a path, identifier, selection or string) rather than prose?
pub fn looks_like_simple_expr(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() || t.contains(char::is_whitespace) || t.contains('=') || t.contains(';') {
        return false;
    }
    let first = t.chars().next().unwrap();
    if !(first.is_ascii_alphabetic() || matches!(first, '.' | '/' | '~' | '<' | '"' | '_')) {
        return false;
    }
    if t.ends_with('.') || t.ends_with(',') || t.ends_with(':') {
        return false;
    }
    let p = rnix::Root::parse(t);
    if !p.errors().is_empty() {
        return false;
    }
    matches!(
        p.tree().expr(),
        Some(ast::Expr::PathRel(_) | ast::Expr::PathAbs(_) | ast::Expr::PathHome(_) | ast::Expr::PathSearch(_) | ast::Expr::Ident(_) | ast::Expr::Select(_) | ast::Expr::Str(_))
    )
}

// ───────────────────────── tree building ─────────────────────────

pub struct TreeBuilder<'a> {
    parsed: &'a Parsed,
}

impl<'a> TreeBuilder<'a> {
    pub fn new(parsed: &'a Parsed) -> Self {
        Self { parsed }
    }

    fn base(&self, kind: NodeKind, entry: TextRange, value: TextRange) -> Node {
        Node {
            id: node_id(entry),
            kind,
            label: None,
            span: self.parsed.span(entry),
            value_span: self.parsed.span(value),
            value: None,
            source: None,
            children: Vec::new(),
            comment: None,
            with_namespace: None,
            interpolated: false,
            disabled: false,
            editable: false,
        }
    }

    pub fn build_root(&self, body: Option<ast::Expr>) -> Node {
        match body {
            Some(expr) => self.convert(&expr, None, None, None),
            None => {
                let whole = TextRange::new(0.into(), (self.parsed.source.len() as u32).into());
                let mut n = self.base(NodeKind::Code, whole, whole);
                n.source = Some(self.parsed.source.clone());
                n
            }
        }
    }

    /// Convert an expression. `entry` is the enclosing binding/item range used
    /// for `span` (defaults to the expression's own range).
    pub fn convert(&self, expr: &ast::Expr, label: Option<String>, entry: Option<TextRange>, comment: Option<String>) -> Node {
        let vrange = expr.syntax().text_range();
        let erange = entry.unwrap_or(vrange);
        let mut node = match expr {
            ast::Expr::AttrSet(set) => {
                let mut n = self.base(NodeKind::Attrset, erange, vrange);
                n.children = self.entries(set);
                n
            }
            ast::Expr::LetIn(li) => {
                let mut n = self.base(NodeKind::Letin, erange, vrange);
                n.children = self.entries(li);
                if let Some(body) = li.body() {
                    n.children.push(self.convert(&body, Some("in".to_string()), None, None));
                }
                n
            }
            ast::Expr::List(list) => {
                let mut n = self.base(NodeKind::List, erange, vrange);
                n.children = self.list_items(list);
                n
            }
            ast::Expr::With(w) => {
                let ns = w.namespace().map(|e| e.syntax().text().to_string());
                match w.body() {
                    Some(body) => {
                        let mut inner = self.convert(&body, None, Some(erange), None);
                        inner.value_span = self.parsed.span(vrange);
                        inner.id = node_id(erange);
                        inner.with_namespace = ns;
                        inner
                    }
                    None => self.code(erange, vrange),
                }
            }
            ast::Expr::Paren(p) => match p.expr() {
                Some(inner) => {
                    let mut n = self.convert(&inner, None, Some(erange), None);
                    n.value_span = self.parsed.span(vrange);
                    n.id = node_id(erange);
                    n
                }
                None => self.code(erange, vrange),
            },
            ast::Expr::Ident(id) => {
                let text = id.syntax().text().to_string();
                let kind = match text.as_str() {
                    "true" | "false" => NodeKind::Bool,
                    "null" => NodeKind::Null,
                    _ => NodeKind::Ref,
                };
                let mut n = self.base(kind, erange, vrange);
                n.value = Some(text);
                n.editable = true;
                n
            }
            ast::Expr::Select(sel) => {
                if sel.or_token().is_some() {
                    self.code(erange, vrange)
                } else {
                    let mut n = self.base(NodeKind::Ref, erange, vrange);
                    n.value = Some(sel.syntax().text().to_string());
                    n.editable = true;
                    n
                }
            }
            ast::Expr::Literal(lit) => {
                let text = lit.syntax().text().to_string();
                let kind = if text.parse::<i64>().is_ok() {
                    NodeKind::Int
                } else if text.parse::<f64>().is_ok() {
                    NodeKind::Float
                } else {
                    NodeKind::Ref
                };
                let mut n = self.base(kind, erange, vrange);
                n.value = Some(text);
                n.editable = true;
                n
            }
            ast::Expr::Str(s) => {
                let mut interpolated = false;
                let mut literal = String::new();
                for part in s.normalized_parts() {
                    match part {
                        ast::InterpolPart::Literal(t) => literal.push_str(&t),
                        ast::InterpolPart::Interpolation(_) => interpolated = true,
                    }
                }
                let mut n = self.base(NodeKind::String, erange, vrange);
                n.interpolated = interpolated;
                n.value = Some(if interpolated { s.syntax().text().to_string() } else { literal });
                n.source = Some(s.syntax().text().to_string());
                n.editable = !interpolated;
                n
            }
            ast::Expr::PathRel(_) | ast::Expr::PathAbs(_) | ast::Expr::PathHome(_) | ast::Expr::PathSearch(_) => {
                let mut n = self.base(NodeKind::Path, erange, vrange);
                n.value = Some(expr.syntax().text().to_string());
                n.editable = true;
                n
            }
            ast::Expr::BinOp(op) => {
                let kind = match op.operator() {
                    Some(ast::BinOpKind::Concat) => Some(NodeKind::Concat),
                    Some(ast::BinOpKind::Update) => Some(NodeKind::Merge),
                    _ => None,
                };
                match (kind, op.lhs(), op.rhs()) {
                    (Some(k), Some(l), Some(r)) => {
                        let mut n = self.base(k, erange, vrange);
                        n.children = vec![self.convert(&l, None, None, None), self.convert(&r, None, None, None)];
                        n
                    }
                    _ => self.code(erange, vrange),
                }
            }
            ast::Expr::Lambda(_) => {
                let mut n = self.base(NodeKind::Function, erange, vrange);
                n.source = Some(expr.syntax().text().to_string());
                n
            }
            _ => self.code(erange, vrange),
        };
        node.label = label;
        node.comment = comment;
        node
    }

    fn code(&self, erange: TextRange, vrange: TextRange) -> Node {
        let mut n = self.base(NodeKind::Code, erange, vrange);
        n.source = Some(self.parsed.text(vrange).to_string());
        n
    }

    fn entries<T: HasEntry>(&self, set: &T) -> Vec<Node> {
        let mut out = Vec::new();
        for entry in set.entries() {
            match entry {
                ast::Entry::AttrpathValue(av) => {
                    let key = av.attrpath().map(|p| attrpath_text(&p)).unwrap_or_default();
                    let erange = av.syntax().text_range();
                    // code-like leading comments are shown as disabled settings instead (below)
                    let comment = join_comments(leading_comments(av.syntax(), true), trailing_comment(av.syntax()));
                    match av.value() {
                        Some(v) => out.push(self.convert(&v, Some(key), Some(erange), comment)),
                        None => {
                            let mut n = self.code(erange, erange);
                            n.label = Some(key);
                            n.comment = comment;
                            out.push(n);
                        }
                    }
                }
                ast::Entry::Inherit(inh) => {
                    let r = inh.syntax().text_range();
                    let mut n = self.base(NodeKind::Inherit, r, r);
                    n.value = Some(inh.syntax().text().to_string());
                    n.source = n.value.clone();
                    n.label = Some("inherit".to_string());
                    n.comment = join_comments(leading_comments(inh.syntax(), false), trailing_comment(inh.syntax()));
                    out.push(n);
                }
            }
        }
        // Commented-out settings ("# key = value;" or a whole "# key = {" … "# };"
        // block on consecutive comment lines): shown as disabled, re-enabled
        // with one click (EditOp::ToggleDisabled on the comment block).
        for (_, _, lines) in comment_groups(set.syntax()) {
            // Greedy: the longest run of consecutive comment lines that parses as
            // exactly one setting becomes a disabled node; prose lines and
            // unrelated settings around it are left alone.
            let mut i = 0;
            while i < lines.len() {
                let mut found = None;
                for j in (i..lines.len()).rev() {
                    let range = TextRange::new(lines[i].0.start(), lines[j].0.end());
                    let text = lines[i..=j].iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>().join("\n");
                    if let Some(n) = self.disabled_binding(range, &text) {
                        found = Some((j, n));
                        break;
                    }
                }
                match found {
                    Some((j, n)) => {
                        out.push(n);
                        i = j + 1;
                    }
                    None => i += 1,
                }
            }
        }
        out.sort_by_key(|n| n.span.start);
        out
    }

    fn disabled_binding(&self, range: TextRange, text: &str) -> Option<Node> {
        if !text.contains('=') || text.trim().starts_with('{') {
            return None;
        }
        let wrapped = format!("{{\n{text}\n}}");
        let p = rnix::Root::parse(&wrapped);
        if !p.errors().is_empty() {
            return None;
        }
        let expr = p.tree().expr()?;
        let ast::Expr::AttrSet(set) = expr else { return None };
        let mut entries = set.entries();
        let Some(ast::Entry::AttrpathValue(av)) = entries.next() else { return None };
        if entries.next().is_some() {
            return None;
        }
        let key = av.attrpath().map(|p| attrpath_text(&p))?;
        let kind = match av.value() {
            Some(ast::Expr::AttrSet(_)) => NodeKind::Attrset,
            Some(ast::Expr::List(_)) => NodeKind::List,
            Some(ast::Expr::Str(_)) => NodeKind::String,
            Some(ast::Expr::Ident(ref i)) if matches!(i.syntax().text().to_string().as_str(), "true" | "false") => NodeKind::Bool,
            Some(ast::Expr::Literal(_)) => NodeKind::Int,
            _ => NodeKind::Code,
        };
        let mut n = self.base(kind, range, range);
        n.label = Some(key);
        n.value = Some(av.value().map(|v| v.syntax().text().to_string()).unwrap_or_default());
        n.source = Some(text.to_string());
        n.disabled = true;
        Some(n)
    }
}

/// Runs of consecutive `#` line comments directly inside `node` (separated by
/// at most one newline), as (range covering the run, text with the `#`
/// markers removed, lines joined by newlines).
pub fn comment_groups(node: &SyntaxNode) -> Vec<(TextRange, String, Vec<(TextRange, String)>)> {
    let mut groups = Vec::new();
    let mut cur: Option<(TextRange, Vec<(TextRange, String)>)> = None;
    let flush = |cur: &mut Option<(TextRange, Vec<(TextRange, String)>)>, groups: &mut Vec<(TextRange, String, Vec<(TextRange, String)>)>| {
        if let Some((r, lines)) = cur.take() {
            let text = lines.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>().join("\n");
            groups.push((r, text, lines));
        }
    };
    for el in node.children_with_tokens() {
        match el {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::TOKEN_COMMENT && t.text().starts_with('#') => {
                let line = (t.text_range(), strip_hash(t.text()));
                match cur.as_mut() {
                    Some((r, lines)) => {
                        *r = TextRange::new(r.start(), t.text_range().end());
                        lines.push(line);
                    }
                    None => cur = Some((t.text_range(), vec![line])),
                }
            }
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::TOKEN_WHITESPACE && t.text().matches('\n').count() <= 1 => {}
            _ => flush(&mut cur, &mut groups),
        }
    }
    flush(&mut cur, &mut groups);
    groups
}

/// `# foo` → `foo`, `#foo` → `foo`, `#` → ``.
pub fn strip_hash(line: &str) -> String {
    let rest = line.strip_prefix('#').unwrap_or(line);
    rest.strip_prefix(' ').unwrap_or(rest).to_string()
}

#[cfg(test)]
mod disabled_binding_tests {
    use super::*;

    #[test]
    fn commented_out_group_and_setting_show_as_disabled() {
        let src = "{ config, pkgs, ... }: {\n  services.openssh.enable = true;\n  # services.printing = {\n  #   enable = true;\n  #   drivers = [ pkgs.gutenprint ];\n  # };\n  # networking.firewall.enable = false;\n  # just a note, not code\n  users.users.v.isNormalUser = true;\n}\n";
        let v = analyze_file("t.nix", src.to_string());
        let labels: Vec<(String, bool)> = v.root.children.iter().map(|c| (c.label.clone().unwrap_or_default(), c.disabled)).collect();
        assert!(labels.contains(&("services.printing".to_string(), true)), "{labels:?}");
        assert!(labels.contains(&("networking.firewall.enable".to_string(), true)), "{labels:?}");
        assert!(labels.contains(&("services.openssh.enable".to_string(), false)));
        assert!(!labels.iter().any(|(l, _)| l.contains("note")), "{labels:?}");
        let grp = v.root.children.iter().find(|c| c.label.as_deref() == Some("services.printing")).unwrap();
        assert_eq!(grp.kind, NodeKind::Attrset);
        assert!(grp.source.as_deref().unwrap().contains("drivers = [ pkgs.gutenprint ];"));
        // order follows the file
        let idx = |l: &str| v.root.children.iter().position(|c| c.label.as_deref() == Some(l)).unwrap();
        assert!(idx("services.openssh.enable") < idx("services.printing") && idx("services.printing") < idx("users.users.v.isNormalUser"));
    }
}

impl<'a> TreeBuilder<'a> {

    fn list_items(&self, list: &ast::List) -> Vec<Node> {
        let mut items: Vec<(usize, Node)> = Vec::new();
        for item in list.items() {
            let r = item.syntax().text_range();
            let comment = join_comments(leading_comments(item.syntax(), true), trailing_comment(item.syntax()));
            let n = self.convert(&item, None, Some(r), comment);
            items.push((r.start().into(), n));
        }
        // Commented-out items.
        for el in list.syntax().children_with_tokens() {
            if let NodeOrToken::Token(t) = el {
                if t.kind() == SyntaxKind::TOKEN_COMMENT {
                    if let Some(n) = self.disabled_item(&t) {
                        items.push((t.text_range().start().into(), n));
                    }
                }
            }
        }
        items.sort_by_key(|(off, _)| *off);
        items.into_iter().map(|(_, n)| n).collect()
    }

    fn disabled_item(&self, token: &SyntaxToken) -> Option<Node> {
        let text = clean_comment(token.text());
        if !looks_like_simple_expr(&text) {
            return None;
        }
        let p = rnix::Root::parse(&text);
        let expr = p.tree().expr()?;
        let kind = match expr {
            ast::Expr::Str(_) => NodeKind::String,
            ast::Expr::Ident(ref i) => match i.syntax().text().to_string().as_str() {
                "true" | "false" => NodeKind::Bool,
                "null" => NodeKind::Null,
                _ => NodeKind::Ref,
            },
            ast::Expr::Select(_) => NodeKind::Ref,
            _ => NodeKind::Path,
        };
        let r = token.text_range();
        let mut n = self.base(kind, r, r);
        n.value = Some(if let ast::Expr::Str(s) = &expr {
            s.normalized_parts()
                .into_iter()
                .filter_map(|p| if let ast::InterpolPart::Literal(t) = p { Some(t) } else { None })
                .collect::<String>()
        } else {
            text.clone()
        });
        n.source = Some(text);
        n.disabled = true;
        Some(n)
    }
}

pub fn attrpath_text(p: &ast::Attrpath) -> String {
    p.attrs().map(|a| a.syntax().text().to_string()).collect::<Vec<_>>().join(".")
}

fn attrpath_parts(p: &ast::Attrpath) -> Vec<String> {
    p.attrs().map(|a| a.syntax().text().to_string().trim_matches('"').to_string()).collect()
}

// ───────────────────────── whole-file analysis ─────────────────────────

pub fn analyze_file(rel_path: &str, source: String) -> NixFileView {
    let parsed = Parsed::parse(source);
    let (args, has_ellipsis, body) = parsed.module_body();
    let builder = TreeBuilder::new(&parsed);
    let root = builder.build_root(body);
    NixFileView {
        rel_path: rel_path.to_string(),
        version: parsed.version(),
        parse_errors: parsed.parse_errors(),
        args,
        has_ellipsis,
        root,
        line_count: parsed.source.lines().count() as u32,
        source: parsed.source,
    }
}

/// First comment block in the file, used as a description.
pub fn file_description(parsed: &Parsed) -> Option<String> {
    let mut out = Vec::new();
    for el in parsed.root.syntax().children_with_tokens() {
        match el {
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::TOKEN_COMMENT => out.push(clean_comment(t.text())),
            NodeOrToken::Token(t) if t.kind() == SyntaxKind::TOKEN_WHITESPACE => {
                if !out.is_empty() && t.text().matches('\n').count() >= 2 {
                    break;
                }
            }
            _ => break,
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join(" "))
    }
}

/// Option namespaces touched by the module body, e.g. "services.docker".
pub fn namespaces(parsed: &Parsed) -> Vec<String> {
    let (_, _, body) = parsed.module_body();
    let mut out = Vec::new();
    fn collect(expr: &ast::Expr, out: &mut Vec<String>) {
        match expr {
            ast::Expr::AttrSet(set) => {
                for av in set.attrpath_values() {
                    let Some(p) = av.attrpath() else { continue };
                    let parts = attrpath_parts(&p);
                    if parts.first().map(|s| s.as_str()) == Some("imports") {
                        continue;
                    }
                    let ns = parts.iter().take(2).cloned().collect::<Vec<_>>().join(".");
                    if !ns.is_empty() && !out.contains(&ns) {
                        out.push(ns);
                    }
                }
            }
            ast::Expr::LetIn(li) => {
                if let Some(b) = li.body() {
                    collect(&b, out);
                }
            }
            ast::Expr::Paren(p) => {
                if let Some(e) = p.expr() {
                    collect(&e, out);
                }
            }
            _ => {}
        }
    }
    if let Some(b) = body {
        collect(&b, &mut out);
    }
    out.sort();
    out
}

/// Every path literal in the file, classified as an import or a reference.
pub fn imports(parsed: &Parsed, file_rel: &str, config_root: &Path) -> Vec<ImportRef> {
    let file_dir = Path::new(file_rel).parent().unwrap_or(Path::new("")).to_path_buf();
    let mut out = Vec::new();
    let mut in_imports: Vec<TextRange> = Vec::new();

    // Find `imports = ...` lists.
    for node in parsed.root.syntax().descendants() {
        if let Some(av) = ast::AttrpathValue::cast(node.clone()) {
            let is_imports = av.attrpath().map(|p| attrpath_parts(&p) == ["imports"]).unwrap_or(false);
            if is_imports {
                if let Some(v) = av.value() {
                    in_imports.push(v.syntax().text_range());
                }
            }
        }
    }
    let inside_imports = |r: TextRange| in_imports.iter().any(|ir| ir.contains_range(r));

    for node in parsed.root.syntax().descendants() {
        let is_path = matches!(node.kind(), SyntaxKind::NODE_PATH_REL | SyntaxKind::NODE_PATH_ABS | SyntaxKind::NODE_PATH_HOME);
        if !is_path {
            continue;
        }
        let raw = node.text().to_string();
        if raw.contains("${") {
            continue;
        }
        let r = node.text_range();
        let via = if inside_imports(r) { ImportVia::Imports } else { ImportVia::Reference };
        if let Some((target, exists)) = resolve_target(&raw, &file_dir, config_root) {
            out.push(ImportRef { target, raw, via, enabled: true, exists, line: parsed.line_of(r.start().into()), node_id: node_id(r) });
        }
    }

    // Commented-out imports inside import lists.
    for node in parsed.root.syntax().descendants() {
        if node.kind() != SyntaxKind::NODE_LIST || !inside_imports(node.text_range()) {
            continue;
        }
        for el in node.children_with_tokens() {
            if let NodeOrToken::Token(t) = el {
                if t.kind() != SyntaxKind::TOKEN_COMMENT {
                    continue;
                }
                let text = clean_comment(t.text());
                if !looks_like_simple_expr(&text) || !(text.starts_with("./") || text.starts_with("../") || text.starts_with('/')) {
                    continue;
                }
                let r = t.text_range();
                if let Some((target, exists)) = resolve_target(&text, &file_dir, config_root) {
                    out.push(ImportRef { target, raw: text, via: ImportVia::Imports, enabled: false, exists, line: parsed.line_of(r.start().into()), node_id: node_id(r) });
                }
            }
        }
    }
    out.sort_by_key(|i| i.line);
    out
}

fn resolve_target(raw: &str, file_dir: &Path, config_root: &Path) -> Option<(String, bool)> {
    let rel: PathBuf = if raw.starts_with('/') {
        // Absolute: only track if it lives under the config root.
        let p = Path::new(raw);
        p.strip_prefix(config_root).ok()?.to_path_buf()
    } else if raw.starts_with('~') {
        return None;
    } else {
        normalize(&file_dir.join(raw))
    };
    if rel.components().next().map(|c| matches!(c, Component::ParentDir)).unwrap_or(false) {
        return None; // escapes the config root
    }
    let full = config_root.join(&rel);
    let (rel, exists) = if full.is_dir() {
        let d = rel.join("default.nix");
        let e = config_root.join(&d).is_file();
        (if e { d } else { rel }, e)
    } else {
        (rel, full.is_file())
    };
    Some((rel.to_string_lossy().to_string(), exists))
}

pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Statically find `nixosConfigurations.<name>` in a flake.nix.
pub fn flake_hosts_static(parsed: &Parsed) -> Vec<String> {
    let mut hosts = Vec::new();
    for node in parsed.root.syntax().descendants() {
        let Some(av) = ast::AttrpathValue::cast(node) else { continue };
        let Some(p) = av.attrpath() else { continue };
        let parts = attrpath_parts(&p);
        if let Some(i) = parts.iter().position(|s| s == "nixosConfigurations") {
            if let Some(name) = parts.get(i + 1) {
                if !name.contains("${") && !hosts.contains(name) {
                    hosts.push(name.clone());
                }
            } else if let Some(ast::Expr::AttrSet(set)) = av.value() {
                for inner in set.attrpath_values() {
                    if let Some(ip) = inner.attrpath() {
                        let name = attrpath_parts(&ip).join(".");
                        if !name.contains("${") && !hosts.contains(&name) {
                            hosts.push(name);
                        }
                    }
                }
            }
        }
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{ config, pkgs, pkgs-unstable, ... }:

{
  imports = [
    ./hardware-configuration.nix
    ./audio.nix
    # ./nvidia.nix
  ];

  # Enable docker
  virtualisation.docker.enable = true;
  networking.hostName = "box"; # the name
  environment.systemPackages = with pkgs; [
    git
    # neofetch
    pkgs-unstable.steam
  ];
  nix.settings.download-buffer-size = 4294967296;
}
"#;

    #[test]
    fn builds_tree() {
        let v = analyze_file("configuration.nix", SAMPLE.to_string());
        assert!(v.parse_errors.is_empty());
        assert_eq!(v.args, vec!["config", "pkgs", "pkgs-unstable"]);
        assert!(v.has_ellipsis);
        assert_eq!(v.root.kind, NodeKind::Attrset);
        let labels: Vec<_> = v.root.children.iter().map(|c| c.label.clone().unwrap()).collect();
        assert_eq!(labels, vec!["imports", "virtualisation.docker.enable", "networking.hostName", "environment.systemPackages", "nix.settings.download-buffer-size"]);
        let docker = &v.root.children[1];
        assert_eq!(docker.kind, NodeKind::Bool);
        assert_eq!(docker.comment.as_deref(), Some("Enable docker"));
        let host = &v.root.children[2];
        assert_eq!(host.value.as_deref(), Some("box"));
        assert_eq!(host.comment.as_deref(), Some("the name"));
        let pkgs = &v.root.children[3];
        assert_eq!(pkgs.kind, NodeKind::List);
        assert_eq!(pkgs.with_namespace.as_deref(), Some("pkgs"));
        assert_eq!(pkgs.children.len(), 3);
        assert!(pkgs.children[1].disabled);
        assert_eq!(pkgs.children[1].value.as_deref(), Some("neofetch"));
        let imports = &v.root.children[0];
        assert_eq!(imports.children.len(), 3);
        assert!(imports.children[2].disabled);
        assert_eq!(v.root.children[4].kind, NodeKind::Int);
    }

    #[test]
    fn extracts_imports_and_namespaces() {
        let parsed = Parsed::parse(SAMPLE.to_string());
        let ns = namespaces(&parsed);
        assert_eq!(ns, vec!["environment.systemPackages", "networking.hostName", "nix.settings", "virtualisation.docker"]);
        let imps = imports(&parsed, "configuration.nix", Path::new("/nonexistent"));
        let targets: Vec<_> = imps.iter().map(|i| (i.target.as_str(), i.enabled)).collect();
        assert_eq!(targets, vec![("hardware-configuration.nix", true), ("audio.nix", true), ("nvidia.nix", false)]);
    }

    #[test]
    fn finds_flake_hosts() {
        let flake = r#"{ outputs = { nixpkgs, ... }: { nixosConfigurations.main = nixpkgs.lib.nixosSystem {}; nixosConfigurations = { laptop = 1; }; }; }"#;
        let parsed = Parsed::parse(flake.to_string());
        assert_eq!(flake_hosts_static(&parsed), vec!["main", "laptop"]);
    }

    #[test]
    fn trailing_comment_does_not_leak_to_next_line() {
        let src = "{\n  a = true; # about a\n  b = false;\n  # about c\n  c = 1;\n}";
        let v = analyze_file("t.nix", src.to_string());
        assert_eq!(v.root.children[0].comment.as_deref(), Some("about a"));
        assert_eq!(v.root.children[1].comment, None);
        assert_eq!(v.root.children[2].comment.as_deref(), Some("about c"));
    }

    #[test]
    fn simple_expr_heuristic() {
        assert!(looks_like_simple_expr("./nvidia.nix"));
        assert!(looks_like_simple_expr("pkgs.stacer"));
        assert!(looks_like_simple_expr("neofetch"));
        assert!(!looks_like_simple_expr("Game overlay"));
        assert!(!looks_like_simple_expr("Bootloader."));
        assert!(!looks_like_simple_expr("foo = 1;"));
    }
}
