//! Structural edits: addressing by id, by 1-based position, or by heading
//! text, and sections (a heading plus everything after it up to the next
//! heading of the same or higher level).
//!
//! A **number** is a position among the parent's children (the document's
//! top-level blocks unless a parent is given). A **string** is an id, or
//! failing that the text of a heading (case-insensitive, first match). Ids
//! can't be numbers, so the two never collide.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::CfmlError;

use super::cfml::{breaks_to_spaces, check_new_ids, node_to_value, parse_align_list, validate_nodes, Ctx, Loader};
use super::options::{md_err, MdOptions};
use super::tree::{normalise_inlines, trim_block_edges, Content, Doc, Kind, Node};

/// Where a node lives: an index path from the document's flow (or its
/// footnote definitions).
#[derive(Clone, Debug, PartialEq)]
pub struct Loc {
    pub defs: bool,
    pub path: Vec<usize>,
}

/// A container of children.
#[derive(Clone, Debug, PartialEq)]
pub enum Cont {
    Root,
    Node(Loc),
}

pub enum Target {
    Pos(usize),
    Name(String),
}

pub fn target_of(v: &CfmlValue, what: &str) -> Result<Target, CfmlError> {
    match v {
        CfmlValue::Int(n) => pos(*n as f64, what),
        CfmlValue::Double(d) => pos(*d, what),
        CfmlValue::Null => Err(md_err(format!("[{}] is required: an id, a heading's text, or a position.", what))),
        other => {
            let s = other.as_string();
            let t = s.trim();
            if t.is_empty() {
                return Err(md_err(format!("[{}] is required: an id, a heading's text, or a position.", what)));
            }
            match t.parse::<f64>() {
                Ok(n) => pos(n, what),
                Err(_) => Ok(Target::Name(t.to_string())),
            }
        }
    }
}

fn pos(n: f64, what: &str) -> Result<Target, CfmlError> {
    if n < 1.0 || n.fract() != 0.0 {
        return Err(md_err(format!("[{}] position must be a whole number from 1, not {}.", what, n)));
    }
    Ok(Target::Pos(n as usize))
}

// ---------------------------------------------------------------------------
// navigation
// ---------------------------------------------------------------------------

pub fn node<'a>(doc: &'a Doc, l: &Loc) -> &'a Node {
    let mut v = if l.defs { &doc.definitions } else { &doc.children };
    let (last, init) = l.path.split_last().expect("non-empty path");
    for i in init {
        v = &v[*i].children;
    }
    &v[*last]
}

pub fn node_mut<'a>(doc: &'a mut Doc, l: &Loc) -> &'a mut Node {
    let mut v = if l.defs { &mut doc.definitions } else { &mut doc.children };
    let (last, init) = l.path.split_last().expect("non-empty path");
    for i in init {
        v = &mut v[*i].children;
    }
    &mut v[*last]
}

fn children<'a>(doc: &'a Doc, c: &Cont) -> &'a Vec<Node> {
    match c {
        Cont::Root => &doc.children,
        Cont::Node(l) => &node(doc, l).children,
    }
}

fn children_mut<'a>(doc: &'a mut Doc, c: &Cont) -> &'a mut Vec<Node> {
    match c {
        Cont::Root => &mut doc.children,
        Cont::Node(l) => &mut node_mut(doc, l).children,
    }
}

fn slot(doc: &Doc, c: &Cont) -> Content {
    match c {
        Cont::Root => Content::Flow,
        Cont::Node(l) => node(doc, l).kind.content(),
    }
}

fn single_line(doc: &Doc, c: &Cont) -> bool {
    match c {
        Cont::Root => false,
        Cont::Node(l) => matches!(node(doc, l).kind, Kind::Heading { .. } | Kind::TableCell),
    }
}

/// The container holding `l`, and `l`'s index in it.
fn parent_of(l: &Loc) -> Result<(Cont, usize), CfmlError> {
    let (last, init) = l.path.split_last().expect("non-empty path");
    if init.is_empty() {
        if l.defs {
            return Err(md_err("A footnote definition can't be moved or have blocks put beside it."));
        }
        return Ok((Cont::Root, *last));
    }
    Ok((Cont::Node(Loc { defs: l.defs, path: init.to_vec() }), *last))
}

fn find_id(doc: &Doc, id: &str) -> Option<Loc> {
    fn search(nodes: &[Node], id: &str, path: &mut Vec<usize>) -> bool {
        for (i, n) in nodes.iter().enumerate() {
            path.push(i);
            if n.id.as_deref().map(|x| x.eq_ignore_ascii_case(id)).unwrap_or(false) {
                return true;
            }
            if search(&n.children, id, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    let mut path = Vec::new();
    if search(&doc.children, id, &mut path) {
        return Some(Loc { defs: false, path });
    }
    let mut path = Vec::new();
    if search(&doc.definitions, id, &mut path) {
        return Some(Loc { defs: true, path });
    }
    None
}

/// `"## Payment terms"` and `"payment terms"` both name the heading
/// "Payment terms".
fn heading_key(s: &str) -> String {
    s.trim().trim_start_matches('#').trim().to_lowercase()
}

fn find_heading(doc: &Doc, text: &str) -> Option<Loc> {
    let want = heading_key(text);
    fn search(nodes: &[Node], want: &str, path: &mut Vec<usize>) -> bool {
        for (i, n) in nodes.iter().enumerate() {
            path.push(i);
            if matches!(n.kind, Kind::Heading { .. }) && n.plain_text().trim().to_lowercase() == want {
                return true;
            }
            if matches!(n.kind.content(), Content::Flow | Content::ListItems) && search(&n.children, want, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    let mut path = Vec::new();
    if search(&doc.children, &want, &mut path) {
        Some(Loc { defs: false, path })
    } else {
        None
    }
}

fn resolve_name(doc: &Doc, name: &str) -> Result<Loc, CfmlError> {
    find_id(doc, name)
        .or_else(|| find_heading(doc, name))
        .ok_or_else(|| md_err(format!("No block has the id or heading text [{}].", name)))
}

/// The container a `parent` argument names (absent: the document).
pub fn resolve_parent(doc: &Doc, parent: Option<&CfmlValue>) -> Result<Cont, CfmlError> {
    let Some(p) = parent else { return Ok(Cont::Root) };
    match p {
        CfmlValue::Null => Ok(Cont::Root),
        v if v.as_string().trim().is_empty() => Ok(Cont::Root),
        v => {
            let l = match target_of(v, "parent")? {
                Target::Name(n) => resolve_name(doc, &n)?,
                Target::Pos(p) => locate_pos(doc, &Cont::Root, p, "parent")?,
            };
            let k = &node(doc, &l).kind;
            if k.content() == Content::None {
                return Err(md_err(format!("A {} can't hold other content.", k.type_name())));
            }
            Ok(Cont::Node(l))
        }
    }
}

fn locate_pos(doc: &Doc, c: &Cont, p: usize, what: &str) -> Result<Loc, CfmlError> {
    let len = children(doc, c).len();
    if p == 0 || p > len {
        return Err(md_err(format!(
            "[{}] position {} is out of range: there {} {} block{}.",
            what,
            p,
            if len == 1 { "is" } else { "are" },
            len,
            if len == 1 { "" } else { "s" }
        )));
    }
    Ok(match c {
        Cont::Root => Loc { defs: false, path: vec![p - 1] },
        Cont::Node(l) => {
            let mut path = l.path.clone();
            path.push(p - 1);
            Loc { defs: l.defs, path }
        }
    })
}

/// An existing node, by id, heading text, or position within `parent`.
pub fn resolve(doc: &Doc, target: &CfmlValue, parent: Option<&CfmlValue>, what: &str) -> Result<Loc, CfmlError> {
    match target_of(target, what)? {
        Target::Name(n) => resolve_name(doc, &n),
        Target::Pos(p) => {
            let c = resolve_parent(doc, parent)?;
            locate_pos(doc, &c, p, what)
        }
    }
}

fn id_of(doc: &Doc, l: &Loc) -> Result<String, CfmlError> {
    node(doc, l).id.clone().ok_or_else(|| md_err("That node has no id."))
}

fn loc_by_id(doc: &Doc, id: &str) -> Result<Loc, CfmlError> {
    find_id(doc, id).ok_or_else(|| md_err(format!("No block has the id [{}].", id)))
}

// ---------------------------------------------------------------------------
// content in and out
// ---------------------------------------------------------------------------

/// What a container's ancestry implies for content put into it.
fn context_of(doc: &Doc, c: &Cont) -> Ctx {
    let mut ctx = Ctx::default();
    let Cont::Node(l) = c else { return ctx };
    let mut v = if l.defs { &doc.definitions } else { &doc.children };
    for i in &l.path {
        let n = &v[*i];
        match n.kind {
            Kind::Heading { .. } => ctx.single_line = true,
            Kind::TableCell => {
                ctx.single_line = true;
                ctx.in_cell = true;
            }
            Kind::Link { .. } => ctx.in_link = true,
            _ => {}
        }
        v = &n.children;
    }
    ctx
}

/// Undo what loading content may have done to the document before the
/// content itself was attached: footnote definitions it brought, and the
/// footnotes switch a footnote run turns on.
struct Undo {
    definitions: usize,
    footnotes: bool,
}

impl Undo {
    fn new(doc: &Doc) -> Undo {
        Undo { definitions: doc.definitions.len(), footnotes: doc.footnotes }
    }
    fn apply(self, doc: &mut Doc) {
        doc.definitions.truncate(self.definitions);
        doc.footnotes = self.footnotes;
    }
}

/// Attach already-built nodes to container `c` at `index`: check they fit,
/// check supplied ids, mint the rest, validate them in context, splice. The
/// document is not touched until every check has passed.
pub fn graft(doc: &mut Doc, c: &Cont, index: usize, mut nodes: Vec<Node>) -> Result<Vec<String>, CfmlError> {
    let s = slot(doc, c);
    if s == Content::None {
        return Err(md_err("That node can't hold content."));
    }
    if s != Content::Phrasing {
        trim_block_edges(&mut nodes);
    }
    check_new_ids(doc, &nodes)?;
    validate_nodes(doc, &nodes, s, context_of(doc, c))?;
    // A table stays rectangular: new rows match its columns, and a row's
    // cells change only by replacing the row (or a cell's text with setText).
    if let Cont::Node(l) = c {
        match &node(doc, l).kind {
            Kind::Table { align } => {
                for r in &nodes {
                    if r.children.len() != align.len() {
                        return Err(md_err(format!(
                            "A new table row has {} cell{}; the table has {} column{}.",
                            r.children.len(),
                            if r.children.len() == 1 { "" } else { "s" },
                            align.len(),
                            if align.len() == 1 { "" } else { "s" }
                        )));
                    }
                }
            }
            Kind::TableRow => {
                return Err(md_err(
                    "Cells can't be added to or removed from one row; replace the whole row, or use setText on a cell.",
                ))
            }
            _ => {}
        }
    }
    doc.assign_ids(&mut nodes, s);
    let ids: Vec<String> = nodes.iter().filter_map(|n| n.id.clone()).collect();
    let kids = children_mut(doc, c);
    let at = index.min(kids.len());
    kids.splice(at..at, nodes);
    if s == Content::Phrasing {
        if let Cont::Node(l) = c {
            let n = node_mut(doc, l);
            normalise_inlines(&mut n.children);
            trim_block_edges(std::slice::from_mut(n));
        }
    }
    Ok(ids)
}

/// Load `content` for container `c` and graft it at `index`. On failure the
/// document is as it was.
fn splice(doc: &mut Doc, opts: &MdOptions, c: &Cont, index: usize, content: &CfmlValue) -> Result<Vec<String>, CfmlError> {
    let undo = Undo::new(doc);
    let result = (|| {
        let s = slot(doc, c);
        if s == Content::None {
            return Err(md_err("That node can't hold content."));
        }
        let one_line = single_line(doc, c);
        let nodes = {
            let mut loader = Loader::new(doc, opts);
            if one_line {
                loader.single_line_inlines(content)?
            } else {
                loader.content(s, content)?
            }
        };
        graft(doc, c, index, nodes)
    })();
    if result.is_err() {
        undo.apply(doc);
    }
    result
}

/// Build nodes with `make` and graft them, undoing any side effects of the
/// build if anything fails — the builder verbs' path.
pub fn build_and_graft(
    doc: &mut Doc,
    opts: &MdOptions,
    c: &Cont,
    make: impl FnOnce(&mut Loader) -> Result<Vec<Node>, CfmlError>,
) -> Result<Vec<String>, CfmlError> {
    let undo = Undo::new(doc);
    let result = (|| {
        let nodes = {
            let mut loader = Loader::new(doc, opts);
            make(&mut loader)?
        };
        let at = children(doc, c).len();
        graft(doc, c, at, nodes)
    })();
    if result.is_err() {
        undo.apply(doc);
    }
    result
}

pub fn insert_at(doc: &mut Doc, opts: &MdOptions, position: &CfmlValue, content: &CfmlValue, parent: Option<&CfmlValue>) -> Result<Vec<String>, CfmlError> {
    let c = resolve_parent(doc, parent)?;
    let len = children(doc, &c).len();
    let p = match target_of(position, "position")? {
        Target::Pos(p) => p,
        Target::Name(n) => {
            return Err(md_err(format!("insertAt takes a position (a number), not [{}]; use insertBefore or insertAfter for an id.", n)))
        }
    };
    if p > len + 1 {
        return Err(md_err(format!(
            "Position {} is out of range: there {} {} block{}, so the last position is {}.",
            p,
            if len == 1 { "is" } else { "are" },
            len,
            if len == 1 { "" } else { "s" },
            len + 1
        )));
    }
    splice(doc, opts, &c, p - 1, content)
}

pub fn insert_rel(doc: &mut Doc, opts: &MdOptions, target: &CfmlValue, content: &CfmlValue, parent: Option<&CfmlValue>, before: bool) -> Result<Vec<String>, CfmlError> {
    let l = resolve(doc, target, parent, "target")?;
    let (c, i) = parent_of(&l)?;
    splice(doc, opts, &c, if before { i } else { i + 1 }, content)
}

pub fn append(doc: &mut Doc, opts: &MdOptions, parent: &CfmlValue, content: &CfmlValue, front: bool) -> Result<Vec<String>, CfmlError> {
    let c = resolve_parent(doc, Some(parent))?;
    let at = if front { 0 } else { children(doc, &c).len() };
    splice(doc, opts, &c, at, content)
}

pub fn replace(doc: &mut Doc, opts: &MdOptions, target: &CfmlValue, content: &CfmlValue, parent: Option<&CfmlValue>) -> Result<Vec<String>, CfmlError> {
    let l = resolve(doc, target, parent, "target")?;
    let (c, i) = parent_of(&l)?;
    let ids = splice(doc, opts, &c, i + 1, content)?;
    if let Cont::Node(pl) = &c {
        if matches!(node(doc, pl).kind, Kind::Table { .. }) && children(doc, &c).len() == 1 {
            // Nothing was inserted (empty content), so nothing to undo.
            return Err(md_err("A table's last row can't be replaced with nothing; remove the table instead."));
        }
    }
    children_mut(doc, &c).remove(i);
    Ok(ids)
}

pub fn remove(doc: &mut Doc, target: &CfmlValue, parent: Option<&CfmlValue>) -> Result<(), CfmlError> {
    let l = resolve(doc, target, parent, "target")?;
    if l.defs && l.path.len() == 1 {
        doc.definitions.remove(l.path[0]);
        return Ok(());
    }
    let (c, i) = parent_of(&l)?;
    if let Cont::Node(pl) = &c {
        match &node(doc, pl).kind {
            Kind::TableRow => {
                return Err(md_err("A single cell can't be removed: a table row needs one cell per column."))
            }
            Kind::Table { .. } if node(doc, pl).children.len() == 1 => {
                return Err(md_err("A table's last row can't be removed; remove the table instead."))
            }
            _ => {}
        }
    }
    children_mut(doc, &c).remove(i);
    Ok(())
}

fn child_loc(c: &Cont, i: usize) -> Loc {
    match c {
        Cont::Root => Loc { defs: false, path: vec![i] },
        Cont::Node(l) => {
            let mut path = l.path.clone();
            path.push(i);
            Loc { defs: l.defs, path }
        }
    }
}

fn inside(outer: &Loc, inner: &Loc) -> bool {
    outer.defs == inner.defs && inner.path.len() > outer.path.len() && inner.path.starts_with(&outer.path)
}

fn check_fits(doc: &Doc, c: &Cont, nodes: &[Node]) -> Result<(), CfmlError> {
    let s = slot(doc, c);
    for n in nodes {
        if !n.kind.fits(s) {
            return Err(md_err(format!("A {} can't go where {} are expected.", n.kind.type_name(), s.describe())));
        }
    }
    Ok(())
}

pub fn move_to(doc: &mut Doc, target: &CfmlValue, position: &CfmlValue, parent: Option<&CfmlValue>) -> Result<(), CfmlError> {
    let l = resolve(doc, target, parent, "target")?;
    let dest_id = match resolve_parent(doc, parent)? {
        Cont::Root => None,
        Cont::Node(d) => {
            if d == l || inside(&l, &d) {
                return Err(md_err("A block can't be moved into itself."));
            }
            Some(id_of(doc, &d)?)
        }
    };
    let p = match target_of(position, "position")? {
        Target::Pos(p) => p,
        Target::Name(n) => return Err(md_err(format!("move takes a position (a number), not [{}]; use moveBefore or moveAfter.", n))),
    };
    let (c, i) = parent_of(&l)?;
    let moved = children_mut(doc, &c).remove(i);
    let dest = match dest_id {
        None => Cont::Root,
        Some(id) => Cont::Node(loc_by_id(doc, &id)?),
    };
    check_fits(doc, &dest, std::slice::from_ref(&moved))?;
    let kids = children_mut(doc, &dest);
    if p > kids.len() + 1 {
        return Err(md_err(format!("Position {} is out of range: the last position is {}.", p, kids.len() + 1)));
    }
    kids.insert(p - 1, moved);
    Ok(())
}

pub fn move_rel(doc: &mut Doc, target: &CfmlValue, other: &CfmlValue, before: bool) -> Result<(), CfmlError> {
    let l = resolve(doc, target, None, "target")?;
    let o = resolve(doc, other, None, "other")?;
    if o == l || inside(&l, &o) {
        return Err(md_err("A block can't be moved next to itself or into itself."));
    }
    let other_id = id_of(doc, &o)?;
    let (c, i) = parent_of(&l)?;
    let moved = children_mut(doc, &c).remove(i);
    let o = loc_by_id(doc, &other_id)?;
    let (oc, oi) = parent_of(&o)?;
    check_fits(doc, &oc, std::slice::from_ref(&moved))?;
    children_mut(doc, &oc).insert(if before { oi } else { oi + 1 }, moved);
    Ok(())
}

// ---------------------------------------------------------------------------
// sections
// ---------------------------------------------------------------------------

/// The container of a section, and its `[start, end)` range in it.
fn section_range(doc: &Doc, l: &Loc) -> Result<(Cont, usize, usize), CfmlError> {
    let Kind::Heading { depth } = node(doc, l).kind else {
        return Err(md_err(format!("[{}] is not a heading, so it has no section.", node(doc, l).id.clone().unwrap_or_default())));
    };
    let (c, start) = parent_of(l)?;
    let kids = children(doc, &c);
    let mut end = start + 1;
    while end < kids.len() {
        if let Kind::Heading { depth: d } = kids[end].kind {
            if d <= depth {
                break;
            }
        }
        end += 1;
    }
    Ok((c, start, end))
}

fn resolve_heading(doc: &Doc, heading: &CfmlValue) -> Result<Loc, CfmlError> {
    let l = resolve(doc, heading, None, "heading")?;
    if !matches!(node(doc, &l).kind, Kind::Heading { .. }) {
        return Err(md_err(format!(
            "[{}] is a {}, not a heading.",
            heading.as_string(),
            node(doc, &l).kind.type_name()
        )));
    }
    Ok(l)
}

pub fn section_info(doc: &Doc, heading: &CfmlValue) -> Result<CfmlValue, CfmlError> {
    let l = resolve_heading(doc, heading)?;
    let (c, s, e) = section_range(doc, &l)?;
    let kids = children(doc, &c);
    let h = &kids[s];
    let mut m = ValueMap::default();
    m.insert("id", CfmlValue::string(h.id.clone().unwrap_or_default()));
    if let Kind::Heading { depth } = h.kind {
        m.insert("level", CfmlValue::Int(depth as i64));
    }
    m.insert("text", CfmlValue::string(h.plain_text()));
    m.insert("position", CfmlValue::Int(s as i64 + 1));
    m.insert(
        "ids",
        CfmlValue::array(kids[s..e].iter().filter_map(|n| n.id.clone()).map(CfmlValue::string).collect()),
    );
    Ok(CfmlValue::strukt(m))
}

pub fn append_to_section(doc: &mut Doc, opts: &MdOptions, heading: &CfmlValue, content: &CfmlValue) -> Result<Vec<String>, CfmlError> {
    let l = resolve_heading(doc, heading)?;
    let (c, _, e) = section_range(doc, &l)?;
    splice(doc, opts, &c, e, content)
}

pub fn replace_section(doc: &mut Doc, opts: &MdOptions, heading: &CfmlValue, content: &CfmlValue, keep_heading: bool) -> Result<Vec<String>, CfmlError> {
    let l = resolve_heading(doc, heading)?;
    let (c, s, e) = section_range(doc, &l)?;
    let from = if keep_heading { s + 1 } else { s };
    let ids = splice(doc, opts, &c, e, content)?;
    children_mut(doc, &c).drain(from..e);
    Ok(ids)
}

pub fn remove_section(doc: &mut Doc, heading: &CfmlValue) -> Result<(), CfmlError> {
    let l = resolve_heading(doc, heading)?;
    let (c, s, e) = section_range(doc, &l)?;
    children_mut(doc, &c).drain(s..e);
    Ok(())
}

pub fn move_section(doc: &mut Doc, heading: &CfmlValue, position: &CfmlValue) -> Result<(), CfmlError> {
    let l = resolve_heading(doc, heading)?;
    let (c, s, e) = section_range(doc, &l)?;
    let p = match target_of(position, "position")? {
        Target::Pos(p) => p,
        Target::Name(n) => {
            return Err(md_err(format!("moveSection takes a position (a number), not [{}]; use moveSectionBefore or moveSectionAfter.", n)))
        }
    };
    let block: Vec<Node> = children_mut(doc, &c).drain(s..e).collect();
    let kids = children_mut(doc, &c);
    if p > kids.len() + 1 {
        return Err(md_err(format!("Position {} is out of range: the last position is {}.", p, kids.len() + 1)));
    }
    let at = p - 1;
    kids.splice(at..at, block);
    Ok(())
}

/// Move a section next to `other`. After a heading means after that
/// heading's whole section; after any other block means right after it.
pub fn move_section_rel(doc: &mut Doc, heading: &CfmlValue, other: &CfmlValue, before: bool) -> Result<(), CfmlError> {
    let l = resolve_heading(doc, heading)?;
    let o = resolve(doc, other, None, "other")?;
    let (c, s, e) = section_range(doc, &l)?;
    let within = (s..e).map(|k| child_loc(&c, k)).any(|sl| sl == o || inside(&sl, &o));
    if within {
        return Err(md_err("A section can't be moved next to, or into, a block inside itself."));
    }
    let other_id = id_of(doc, &o)?;
    let block: Vec<Node> = children_mut(doc, &c).drain(s..e).collect();
    let o = loc_by_id(doc, &other_id)?;
    let (oc, oi) = parent_of(&o)?;
    check_fits(doc, &oc, &block)?;
    let at = if before {
        oi
    } else if matches!(node(doc, &o).kind, Kind::Heading { .. }) {
        section_range(doc, &o)?.2
    } else {
        oi + 1
    };
    children_mut(doc, &oc).splice(at..at, block);
    Ok(())
}

pub fn outline(doc: &Doc) -> CfmlValue {
    fn walk(nodes: &[Node], out: &mut Vec<CfmlValue>) {
        for (i, n) in nodes.iter().enumerate() {
            if let Kind::Heading { depth } = n.kind {
                let mut m = ValueMap::default();
                m.insert("id", CfmlValue::string(n.id.clone().unwrap_or_default()));
                m.insert("level", CfmlValue::Int(depth as i64));
                m.insert("text", CfmlValue::string(n.plain_text()));
                m.insert("position", CfmlValue::Int(i as i64 + 1));
                out.push(CfmlValue::strukt(m));
            }
            if matches!(n.kind.content(), Content::Flow | Content::ListItems) {
                walk(&n.children, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&doc.children, &mut out);
    CfmlValue::array(out)
}

// ---------------------------------------------------------------------------
// reading and setting fields
// ---------------------------------------------------------------------------

pub fn get(doc: &Doc, target: &CfmlValue, parent: Option<&CfmlValue>) -> Result<CfmlValue, CfmlError> {
    let l = resolve(doc, target, parent, "target")?;
    Ok(node_to_value(node(doc, &l)))
}

pub fn find(doc: &Doc, ty: Option<&str>, text: Option<&str>, level: Option<u8>) -> CfmlValue {
    let ty = ty.map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty());
    let text = text.map(|t| t.to_lowercase()).filter(|t| !t.is_empty());
    let mut out = Vec::new();
    doc.walk(&mut |n| {
        // Only what can be addressed: blocks, links, images.
        if n.id.is_none() {
            return;
        }
        if let Some(t) = &ty {
            if n.kind.type_name().to_ascii_lowercase() != *t {
                return;
            }
        }
        if let Some(lv) = level {
            match n.kind {
                Kind::Heading { depth } if depth == lv => {}
                _ => return,
            }
        }
        if let Some(t) = &text {
            if !n.plain_text().to_lowercase().contains(t.as_str()) {
                return;
            }
        }
        out.push(node_to_value(n));
    });
    CfmlValue::array(out)
}

const SETTABLE: &str = "heading: level; code: language, meta, value; html: value; list: ordered, start, \
                        spread, marker; listItem: checked; table: align; link: url, title; image: url, alt, title; \
                        any block: id";

pub fn set_field(doc: &mut Doc, target: &CfmlValue, field: &str, value: &CfmlValue) -> Result<(), CfmlError> {
    let l = resolve(doc, target, None, "target")?;
    let n = node_mut(doc, &l);
    let f = field.trim().to_ascii_lowercase();
    let absent = matches!(value, CfmlValue::Null) || value.as_string().is_empty();
    let s = || value.as_string();
    let num = |what: &str| -> Result<u64, CfmlError> {
        let t = value.as_string();
        t.trim()
            .parse::<f64>()
            .ok()
            .filter(|x| *x >= 0.0 && x.fract() == 0.0)
            .map(|x| x as u64)
            .ok_or_else(|| md_err(format!("[{}] must be a whole number, not [{}].", what, t)))
    };
    if f == "id" {
        n.id = Some(s());
        return Ok(());
    }
    let ok = match (&mut n.kind, f.as_str()) {
        (Kind::Heading { depth }, "level" | "depth") => {
            let d = num("level")?;
            if !(1..=6).contains(&d) {
                return Err(md_err(format!("Heading level must be 1 to 6, not {}.", d)));
            }
            *depth = d as u8;
            true
        }
        (Kind::Code { lang, .. }, "language" | "lang") => {
            *lang = (!absent).then(s);
            true
        }
        (Kind::Code { meta, .. }, "meta") => {
            *meta = (!absent).then(s);
            true
        }
        (Kind::Code { value: v, .. }, "value" | "text") | (Kind::Html { value: v }, "value" | "markup") => {
            *v = s();
            true
        }
        (Kind::List { ordered, .. }, "ordered") => {
            *ordered = value.is_true();
            true
        }
        (Kind::List { start, .. }, "start") => {
            *start = num("start")?;
            true
        }
        (Kind::List { spread, .. }, "spread") => {
            *spread = value.is_true();
            true
        }
        (Kind::List { marker, .. }, "marker") => {
            *marker = if absent {
                None
            } else {
                let t = s();
                match t.trim() {
                    m @ ("-" | "*" | "+" | "." | ")") => m.chars().next(),
                    other => return Err(md_err(format!("List marker must be one of - * + . ), not [{}].", other))),
                }
            };
            true
        }
        (Kind::ListItem { checked }, "checked") => {
            *checked = if absent { None } else { Some(value.is_true()) };
            true
        }
        (Kind::Table { align }, "align") => {
            let a = parse_align_list(value)?;
            if a.len() != align.len() {
                return Err(md_err(format!("This table has {} columns; [align] gave {}.", align.len(), a.len())));
            }
            *align = a;
            true
        }
        (Kind::Link { url, .. } | Kind::Image { url, .. }, "url" | "href" | "src") => {
            *url = s();
            true
        }
        (Kind::Link { title, .. } | Kind::Image { title, .. }, "title") => {
            *title = (!absent).then(s);
            true
        }
        (Kind::Image { alt, .. }, "alt") => {
            *alt = s();
            true
        }
        _ => false,
    };
    if !ok {
        return Err(md_err(format!(
            "A {} has no settable field [{}]. Settable: {}.",
            n.kind.type_name(),
            field,
            SETTABLE
        )));
    }
    Ok(())
}

pub fn set_fields(doc: &mut Doc, target: &CfmlValue, fields: &CfmlValue) -> Result<(), CfmlError> {
    let map = fields.as_struct().unwrap_or_default();
    for (k, v) in map.iter() {
        set_field(doc, target, k.as_str(), v)?;
    }
    Ok(())
}

/// Replace the content of a block that holds text.
pub fn set_text(doc: &mut Doc, opts: &MdOptions, target: &CfmlValue, content: &CfmlValue) -> Result<(), CfmlError> {
    let l = resolve(doc, target, None, "target")?;
    match &node(doc, &l).kind {
        Kind::Code { .. } | Kind::Html { .. } => {
            let v = content.as_string();
            match &mut node_mut(doc, &l).kind {
                Kind::Code { value, .. } | Kind::Html { value } => *value = v,
                _ => {}
            }
            Ok(())
        }
        Kind::Image { .. } => {
            let v = content.as_string();
            if let Kind::Image { alt, .. } = &mut node_mut(doc, &l).kind {
                *alt = v;
            }
            Ok(())
        }
        Kind::ListItem { .. } => {
            let inl = Loader::new(doc, opts).inlines(content)?;
            let item = node_mut(doc, &l);
            match item.children.first_mut() {
                Some(p) if matches!(p.kind, Kind::Paragraph) => p.children = inl,
                _ => item.children.insert(0, Node::with_children(Kind::Paragraph, inl)),
            }
            Ok(())
        }
        k if k.content() == Content::Phrasing => {
            let one_line = matches!(k, Kind::Heading { .. } | Kind::TableCell);
            let mut inl = Loader::new(doc, opts).inlines(content)?;
            if one_line {
                breaks_to_spaces(&mut inl);
                normalise_inlines(&mut inl);
            }
            let mut inl = inl;
            doc.assign_ids(&mut inl, Content::Phrasing);
            node_mut(doc, &l).children = inl;
            Ok(())
        }
        k => Err(md_err(format!(
            "setText works on a paragraph, heading, table cell, list item, link, code or html block; [{}] is a {}.",
            target.as_string(),
            k.type_name()
        ))),
    }
}
