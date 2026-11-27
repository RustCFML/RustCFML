//! XPath 1.0 evaluator for `XmlSearch()`.
//!
//! Evaluates a practical XPath 1.0 subset over the CFML XML DOM produced by
//! `xmlParse` / `xmlNew` (structs with `xmlName`, `xmlChildren`, `xmlAttributes`,
//! `xmlText`). No third-party XPath crate — keeps the `xml` feature on
//! `quick-xml` alone and avoids THIRD-PARTY.txt churn.
//!
//! Unsupported or malformed syntax throws a clear runtime error (never a silent
//! empty array). See GH #548.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

/// `XmlSearch(xml, xpath)` — evaluate `xpath` against `xml` (document or node).
pub fn xml_search(doc: &CfmlValue, expr: &str) -> CfmlResult {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err(CfmlError::runtime(
            "XmlSearch: XPath expression must not be empty".to_string(),
        ));
    }

    let dom = Dom::from_cfml(doc)?;
    let ast = parse_xpath(expr)?;
    let ns = dom.collect_ns_bindings();
    let start = dom.context_roots();
    let mut ctx = EvalContext {
        dom: &dom,
        position: 1,
        size: 1,
        ns,
    };
    let value = eval_expr(&ast, &start, &mut ctx)?;
    Ok(value_to_cfml(value, ctx.dom))
}

// ---------------------------------------------------------------------------
// Internal DOM
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NodeKind {
    Document,
    Element,
    Attribute,
    Text,
}

#[derive(Clone, Debug)]
struct DomNode {
    kind: NodeKind,
    qname: String,
    local_name: String,
    namespace_uri: String,
    value: String,
    parent: Option<usize>,
    children: Vec<usize>,
    attributes: Vec<usize>,
    cfml: CfmlValue,
    ns_bindings: HashMap<String, String>,
    doc_order: usize,
}

struct Dom {
    nodes: Vec<DomNode>,
    roots: Vec<usize>,
}

impl Dom {
    fn from_cfml(doc: &CfmlValue) -> Result<Self, CfmlError> {
        let mut dom = Dom {
            nodes: Vec::new(),
            roots: Vec::new(),
        };

        match doc {
            CfmlValue::Struct(s) => {
                let xml_type = s.get("xmlType").map(|v| v.as_string()).unwrap_or_default();
                if xml_type.eq_ignore_ascii_case("DOCUMENT") || s.get("xmlRoot").is_some() {
                    let doc_idx = dom.alloc(
                        NodeKind::Document,
                        String::new(),
                        String::new(),
                        String::new(),
                        String::new(),
                        None,
                        doc.clone(),
                        HashMap::new(),
                    );
                    if let Some(root) = s.get("xmlRoot") {
                        let child = dom.build_element(&root, Some(doc_idx), HashMap::new())?;
                        dom.nodes[doc_idx].children.push(child);
                    }
                    dom.roots.push(doc_idx);
                } else if s.get("xmlName").is_some() {
                    let idx = dom.build_element(doc, None, HashMap::new())?;
                    dom.roots.push(idx);
                } else {
                    return Err(CfmlError::runtime(
                        "XmlSearch: first argument must be an XML document or element".to_string(),
                    ));
                }
            }
            _ => {
                return Err(CfmlError::runtime(
                    "XmlSearch: first argument must be an XML document or element".to_string(),
                ));
            }
        }

        if let Some(&root) = dom.roots.first() {
            let mut order = 0usize;
            dom.assign_doc_order(root, &mut order);
        }
        Ok(dom)
    }

    #[allow(clippy::too_many_arguments)]
    fn alloc(
        &mut self,
        kind: NodeKind,
        qname: String,
        local_name: String,
        namespace_uri: String,
        value: String,
        parent: Option<usize>,
        cfml: CfmlValue,
        ns_bindings: HashMap<String, String>,
    ) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(DomNode {
            kind,
            qname,
            local_name,
            namespace_uri,
            value,
            parent,
            children: Vec::new(),
            attributes: Vec::new(),
            cfml,
            ns_bindings,
            doc_order: 0,
        });
        idx
    }

    fn build_element(
        &mut self,
        node: &CfmlValue,
        parent: Option<usize>,
        parent_ns: HashMap<String, String>,
    ) -> Result<usize, CfmlError> {
        let s = match node {
            CfmlValue::Struct(s) => s,
            _ => {
                return Err(CfmlError::runtime(
                    "XmlSearch: malformed XML node (expected struct)".to_string(),
                ));
            }
        };

        let qname = s.get("xmlName").map(|v| v.as_string()).unwrap_or_default();
        let (prefix, local_name) = split_qname(&qname);

        let mut ns_bindings = parent_ns;
        if let Some(CfmlValue::Struct(attrs)) = s.get("xmlAttributes") {
            for (key, val) in attr_pairs(&attrs) {
                if key.eq_ignore_ascii_case("xmlns") {
                    if val.is_empty() {
                        ns_bindings.remove("");
                    } else {
                        ns_bindings.insert(String::new(), val);
                    }
                } else if let Some(rest) = strip_xmlns_prefix(&key) {
                    ns_bindings.insert(rest.to_string(), val);
                }
            }
        }

        let namespace_uri = if let Some(uri) = s.get("xmlNsURI") {
            let u = uri.as_string();
            if u.is_empty() {
                resolve_ns(&ns_bindings, &prefix)
            } else {
                u
            }
        } else {
            resolve_ns(&ns_bindings, &prefix)
        };

        let text = s.get("xmlText").map(|v| v.as_string()).unwrap_or_default();

        let elem_idx = self.alloc(
            NodeKind::Element,
            qname,
            local_name,
            namespace_uri,
            text.clone(),
            parent,
            node.clone(),
            ns_bindings.clone(),
        );

        if let Some(CfmlValue::Struct(attrs)) = s.get("xmlAttributes") {
            for (key, v) in attr_pairs(&attrs) {
                let (aprefix, alocal) = split_qname(&key);
                let auri =
                    if key.eq_ignore_ascii_case("xmlns") || strip_xmlns_prefix(&key).is_some() {
                        "http://www.w3.org/2000/xmlns/".to_string()
                    } else if aprefix.is_empty() {
                        String::new()
                    } else {
                        resolve_ns(&ns_bindings, &aprefix)
                    };

                let mut attr_map = ValueMap::default();
                attr_map.insert("xmlName".to_string(), CfmlValue::string(key.clone()));
                attr_map.insert(
                    "xmlType".to_string(),
                    CfmlValue::string("ATTRIBUTE_NODE".to_string()),
                );
                attr_map.insert("xmlText".to_string(), CfmlValue::string(v.clone()));
                attr_map.insert("xmlValue".to_string(), CfmlValue::string(v.clone()));

                let aidx = self.alloc(
                    NodeKind::Attribute,
                    key,
                    alocal,
                    auri,
                    v,
                    Some(elem_idx),
                    CfmlValue::strukt(attr_map),
                    ns_bindings.clone(),
                );
                self.nodes[elem_idx].attributes.push(aidx);
            }
        }

        if !text.is_empty() {
            let mut tmap = ValueMap::default();
            tmap.insert(
                "xmlName".to_string(),
                CfmlValue::string("#text".to_string()),
            );
            tmap.insert("xmlType".to_string(), CfmlValue::string("TEXT".to_string()));
            tmap.insert("xmlText".to_string(), CfmlValue::string(text.clone()));
            let tidx = self.alloc(
                NodeKind::Text,
                "#text".to_string(),
                "#text".to_string(),
                String::new(),
                text,
                Some(elem_idx),
                CfmlValue::strukt(tmap),
                ns_bindings.clone(),
            );
            self.nodes[elem_idx].children.push(tidx);
        }

        if let Some(CfmlValue::Array(children)) = s.get("xmlChildren") {
            for child in children.iter() {
                let cidx = self.build_element(&child, Some(elem_idx), ns_bindings.clone())?;
                self.nodes[elem_idx].children.push(cidx);
            }
        }

        Ok(elem_idx)
    }

    fn assign_doc_order(&mut self, idx: usize, order: &mut usize) {
        self.nodes[idx].doc_order = *order;
        *order += 1;
        let attrs = self.nodes[idx].attributes.clone();
        for a in attrs {
            self.nodes[a].doc_order = *order;
            *order += 1;
        }
        let children = self.nodes[idx].children.clone();
        for c in children {
            self.assign_doc_order(c, order);
        }
    }

    fn collect_ns_bindings(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for n in &self.nodes {
            if n.kind == NodeKind::Element {
                for (k, v) in &n.ns_bindings {
                    out.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }
        out
    }

    fn context_roots(&self) -> Vec<usize> {
        self.roots.clone()
    }

    fn string_value(&self, idx: usize) -> String {
        match self.nodes[idx].kind {
            NodeKind::Attribute | NodeKind::Text => self.nodes[idx].value.clone(),
            NodeKind::Document | NodeKind::Element => {
                let mut buf = String::new();
                self.collect_text(idx, &mut buf);
                buf
            }
        }
    }

    fn collect_text(&self, idx: usize, buf: &mut String) {
        match self.nodes[idx].kind {
            NodeKind::Text => buf.push_str(&self.nodes[idx].value),
            NodeKind::Element | NodeKind::Document => {
                for &c in &self.nodes[idx].children {
                    self.collect_text(c, buf);
                }
            }
            NodeKind::Attribute => {}
        }
    }
}

fn attr_pairs(attrs: &cfml_common::dynamic::CfmlStruct) -> Vec<(String, String)> {
    let keys = attrs.keys();
    let mut out = Vec::with_capacity(keys.len());
    for key in keys {
        if let Some(val) = attrs.get(key.as_str()) {
            out.push((key, val.as_string()));
        }
    }
    out
}

fn strip_xmlns_prefix(key: &str) -> Option<&str> {
    if key.len() > 6 && key[..6].eq_ignore_ascii_case("xmlns:") {
        Some(&key[6..])
    } else {
        None
    }
}

fn split_qname(qname: &str) -> (String, String) {
    if let Some((p, l)) = qname.split_once(':') {
        (p.to_string(), l.to_string())
    } else {
        (String::new(), qname.to_string())
    }
}

fn resolve_ns(bindings: &HashMap<String, String>, prefix: &str) -> String {
    bindings.get(prefix).cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum XPathValue {
    Nodes(Vec<usize>),
    Bool(bool),
    Number(f64),
    String(String),
}

impl XPathValue {
    fn to_boolean(&self) -> bool {
        match self {
            XPathValue::Bool(b) => *b,
            XPathValue::Number(n) => *n != 0.0 && !n.is_nan(),
            XPathValue::String(s) => !s.is_empty(),
            XPathValue::Nodes(ns) => !ns.is_empty(),
        }
    }

    fn to_number(&self, dom: &Dom) -> f64 {
        match self {
            XPathValue::Number(n) => *n,
            XPathValue::Bool(b) => {
                if *b {
                    1.0
                } else {
                    0.0
                }
            }
            XPathValue::String(s) => parse_xpath_number(s),
            XPathValue::Nodes(ns) => {
                if let Some(&n) = ns.first() {
                    parse_xpath_number(&dom.string_value(n))
                } else {
                    f64::NAN
                }
            }
        }
    }

    fn to_string_value(&self, dom: &Dom) -> String {
        match self {
            XPathValue::String(s) => s.clone(),
            XPathValue::Bool(b) => {
                if *b {
                    "true".to_string()
                } else {
                    "false".to_string()
                }
            }
            XPathValue::Number(n) => format_xpath_number(*n),
            XPathValue::Nodes(ns) => {
                if let Some(&n) = ns.first() {
                    dom.string_value(n)
                } else {
                    String::new()
                }
            }
        }
    }
}

fn parse_xpath_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

fn format_xpath_number(n: f64) -> String {
    if n.is_nan() {
        "NaN".to_string()
    } else if n.is_infinite() {
        if n.is_sign_positive() {
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        }
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

fn value_to_cfml(v: XPathValue, dom: &Dom) -> CfmlValue {
    match v {
        XPathValue::Nodes(ns) => {
            let mut out = Vec::with_capacity(ns.len());
            for idx in ns {
                out.push(dom.nodes[idx].cfml.clone());
            }
            CfmlValue::array(out)
        }
        XPathValue::Bool(b) => CfmlValue::Bool(b),
        XPathValue::Number(n) => {
            if n.is_finite() && n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
                CfmlValue::Int(n as i64)
            } else {
                CfmlValue::Double(n)
            }
        }
        XPathValue::String(s) => CfmlValue::string(s),
    }
}

fn unique_doc_order(dom: &Dom, nodes: &mut Vec<usize>) {
    nodes.sort_by_key(|&i| dom.nodes[i].doc_order);
    nodes.dedup();
}

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Expr {
    Or(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Eq(Box<Expr>, Box<Expr>),
    Ne(Box<Expr>, Box<Expr>),
    Lt(Box<Expr>, Box<Expr>),
    Le(Box<Expr>, Box<Expr>),
    Gt(Box<Expr>, Box<Expr>),
    Ge(Box<Expr>, Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Div(Box<Expr>, Box<Expr>),
    Mod(Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
    Union(Box<Expr>, Box<Expr>),
    Path(PathExpr),
    /// `filterExpr / relativePath` — evaluate filter to nodes, then steps.
    PathFrom(Box<Expr>, PathExpr),
    Filter(Box<Expr>, Vec<Expr>),
    Literal(String),
    Number(f64),
    Function(String, Vec<Expr>),
}

#[derive(Clone, Debug)]
struct PathExpr {
    absolute: bool,
    leading_descendant: bool,
    steps: Vec<Step>,
}

#[derive(Clone, Debug)]
struct Step {
    axis: Axis,
    test: NodeTest,
    predicates: Vec<Expr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Child,
    Descendant,
    DescendantOrSelf,
    Parent,
    /// XPath `self` axis (`.` / `self::`).
    Oneself,
    Attribute,
}

#[derive(Clone, Debug)]
enum NodeTest {
    Name {
        prefix: Option<String>,
        local: String,
    },
    Wildcard,
    PrefixWildcard {
        prefix: String,
    },
    Node,
    Text,
}

// ---------------------------------------------------------------------------
// Lexer
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Slash,
    DoubleSlash,
    Dot,
    DotDot,
    At,
    ColonColon,
    Star,
    Pipe,
    LParen,
    RParen,
    LBrack,
    RBrack,
    Comma,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    Minus,
    Name(String),
    String(String),
    Number(f64),
    Eof,
}

struct Lexer<'a> {
    src: &'a [u8],
    i: usize,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            src: s.as_bytes(),
            i: 0,
        }
    }

    fn peek(&self) -> u8 {
        self.src.get(self.i).copied().unwrap_or(0)
    }

    fn bump(&mut self) -> u8 {
        let c = self.peek();
        if self.i < self.src.len() {
            self.i += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while self.i < self.src.len() && self.src[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn next_tok(&mut self) -> Result<Tok, CfmlError> {
        self.skip_ws();
        if self.i >= self.src.len() {
            return Ok(Tok::Eof);
        }
        let c = self.peek();
        match c {
            b'/' => {
                self.bump();
                if self.peek() == b'/' {
                    self.bump();
                    Ok(Tok::DoubleSlash)
                } else {
                    Ok(Tok::Slash)
                }
            }
            b'.' => {
                self.bump();
                if self.peek() == b'.' {
                    self.bump();
                    Ok(Tok::DotDot)
                } else if self.peek().is_ascii_digit() {
                    self.i -= 1;
                    self.lex_number()
                } else {
                    Ok(Tok::Dot)
                }
            }
            b'@' => {
                self.bump();
                Ok(Tok::At)
            }
            b'*' => {
                self.bump();
                Ok(Tok::Star)
            }
            b'|' => {
                self.bump();
                Ok(Tok::Pipe)
            }
            b'(' => {
                self.bump();
                Ok(Tok::LParen)
            }
            b')' => {
                self.bump();
                Ok(Tok::RParen)
            }
            b'[' => {
                self.bump();
                Ok(Tok::LBrack)
            }
            b']' => {
                self.bump();
                Ok(Tok::RBrack)
            }
            b',' => {
                self.bump();
                Ok(Tok::Comma)
            }
            b'+' => {
                self.bump();
                Ok(Tok::Plus)
            }
            b'-' => {
                self.bump();
                Ok(Tok::Minus)
            }
            b'=' => {
                self.bump();
                Ok(Tok::Eq)
            }
            b'!' => {
                self.bump();
                if self.peek() == b'=' {
                    self.bump();
                    Ok(Tok::Ne)
                } else {
                    Err(CfmlError::runtime(
                        "XmlSearch: invalid XPath: '!' must be followed by '='".to_string(),
                    ))
                }
            }
            b'<' => {
                self.bump();
                if self.peek() == b'=' {
                    self.bump();
                    Ok(Tok::Le)
                } else {
                    Ok(Tok::Lt)
                }
            }
            b'>' => {
                self.bump();
                if self.peek() == b'=' {
                    self.bump();
                    Ok(Tok::Ge)
                } else {
                    Ok(Tok::Gt)
                }
            }
            b'\'' | b'"' => self.lex_string(),
            b'0'..=b'9' => self.lex_number(),
            b':' => {
                self.bump();
                if self.peek() == b':' {
                    self.bump();
                    Ok(Tok::ColonColon)
                } else {
                    Err(CfmlError::runtime(
                        "XmlSearch: invalid XPath: unexpected ':'".to_string(),
                    ))
                }
            }
            _ if is_name_start(c) => self.lex_name(),
            _ => Err(CfmlError::runtime(format!(
                "XmlSearch: invalid XPath: unexpected character '{}'",
                c as char
            ))),
        }
    }

    fn lex_string(&mut self) -> Result<Tok, CfmlError> {
        let quote = self.bump();
        let start = self.i;
        while self.i < self.src.len() && self.src[self.i] != quote {
            self.i += 1;
        }
        if self.i >= self.src.len() {
            return Err(CfmlError::runtime(
                "XmlSearch: invalid XPath: unterminated string literal".to_string(),
            ));
        }
        let s = String::from_utf8_lossy(&self.src[start..self.i]).into_owned();
        self.bump();
        Ok(Tok::String(s))
    }

    fn lex_number(&mut self) -> Result<Tok, CfmlError> {
        let start = self.i;
        while self.peek().is_ascii_digit() {
            self.bump();
        }
        if self.peek() == b'.' {
            self.bump();
            while self.peek().is_ascii_digit() {
                self.bump();
            }
        }
        let s = String::from_utf8_lossy(&self.src[start..self.i]).into_owned();
        let n: f64 = s
            .parse()
            .map_err(|_| CfmlError::runtime(format!("XmlSearch: invalid XPath number '{}'", s)))?;
        Ok(Tok::Number(n))
    }

    fn lex_name(&mut self) -> Result<Tok, CfmlError> {
        let start = self.i;
        self.bump();
        while is_name_char(self.peek()) {
            self.bump();
        }
        if self.peek() == b':' && self.src.get(self.i + 1).copied() != Some(b':') {
            self.bump();
            if self.peek() == b'*' {
                self.bump();
            } else if is_name_start(self.peek()) {
                while is_name_char(self.peek()) {
                    self.bump();
                }
            } else {
                return Err(CfmlError::runtime(
                    "XmlSearch: invalid XPath: expected name after ':'".to_string(),
                ));
            }
        }
        let s = String::from_utf8_lossy(&self.src[start..self.i]).into_owned();
        Ok(Tok::Name(s))
    }
}

fn is_name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.'
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

struct Parser {
    toks: Vec<Tok>,
    i: usize,
}

impl Parser {
    fn new(expr: &str) -> Result<Self, CfmlError> {
        let mut lex = Lexer::new(expr);
        let mut toks = Vec::new();
        loop {
            let t = lex.next_tok()?;
            let done = t == Tok::Eof;
            toks.push(t);
            if done {
                break;
            }
        }
        Ok(Self { toks, i: 0 })
    }

    fn peek(&self) -> &Tok {
        self.toks.get(self.i).unwrap_or(&Tok::Eof)
    }

    fn peek_at(&self, off: usize) -> &Tok {
        self.toks.get(self.i + off).unwrap_or(&Tok::Eof)
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks.get(self.i).cloned().unwrap_or(Tok::Eof);
        if self.i < self.toks.len() {
            self.i += 1;
        }
        t
    }

    fn expect(&mut self, want: &Tok) -> Result<(), CfmlError> {
        let got = self.bump();
        if &got == want {
            Ok(())
        } else {
            Err(CfmlError::runtime(format!(
                "XmlSearch: invalid XPath: expected {:?}, got {:?}",
                want, got
            )))
        }
    }

    fn parse(mut self) -> Result<Expr, CfmlError> {
        let e = self.parse_or()?;
        if self.peek() != &Tok::Eof {
            return Err(CfmlError::runtime(format!(
                "XmlSearch: invalid XPath: unexpected token {:?}",
                self.peek()
            )));
        }
        Ok(e)
    }

    fn parse_or(&mut self) -> Result<Expr, CfmlError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Tok::Name(n) if n == "or") {
            self.bump();
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, CfmlError> {
        let mut left = self.parse_equality()?;
        while matches!(self.peek(), Tok::Name(n) if n == "and") {
            self.bump();
            let right = self.parse_equality()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, CfmlError> {
        let mut left = self.parse_relational()?;
        loop {
            match self.peek() {
                Tok::Eq => {
                    self.bump();
                    let right = self.parse_relational()?;
                    left = Expr::Eq(Box::new(left), Box::new(right));
                }
                Tok::Ne => {
                    self.bump();
                    let right = self.parse_relational()?;
                    left = Expr::Ne(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_relational(&mut self) -> Result<Expr, CfmlError> {
        let mut left = self.parse_additive()?;
        loop {
            match self.peek() {
                Tok::Lt => {
                    self.bump();
                    let right = self.parse_additive()?;
                    left = Expr::Lt(Box::new(left), Box::new(right));
                }
                Tok::Le => {
                    self.bump();
                    let right = self.parse_additive()?;
                    left = Expr::Le(Box::new(left), Box::new(right));
                }
                Tok::Gt => {
                    self.bump();
                    let right = self.parse_additive()?;
                    left = Expr::Gt(Box::new(left), Box::new(right));
                }
                Tok::Ge => {
                    self.bump();
                    let right = self.parse_additive()?;
                    left = Expr::Ge(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, CfmlError> {
        let mut left = self.parse_multiplicative()?;
        loop {
            match self.peek() {
                Tok::Plus => {
                    self.bump();
                    let right = self.parse_multiplicative()?;
                    left = Expr::Add(Box::new(left), Box::new(right));
                }
                Tok::Minus => {
                    self.bump();
                    let right = self.parse_multiplicative()?;
                    left = Expr::Sub(Box::new(left), Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, CfmlError> {
        let mut left = self.parse_unary()?;
        loop {
            match self.peek() {
                Tok::Star => {
                    self.bump();
                    let right = self.parse_unary()?;
                    left = Expr::Mul(Box::new(left), Box::new(right));
                }
                Tok::Name(n) if n == "div" || n == "mod" => {
                    let op = n.clone();
                    self.bump();
                    let right = self.parse_unary()?;
                    left = if op == "div" {
                        Expr::Div(Box::new(left), Box::new(right))
                    } else {
                        Expr::Mod(Box::new(left), Box::new(right))
                    };
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, CfmlError> {
        if matches!(self.peek(), Tok::Minus) {
            self.bump();
            Ok(Expr::Neg(Box::new(self.parse_unary()?)))
        } else {
            self.parse_union()
        }
    }

    fn parse_union(&mut self) -> Result<Expr, CfmlError> {
        let mut left = self.parse_path_expr()?;
        while matches!(self.peek(), Tok::Pipe) {
            self.bump();
            let right = self.parse_path_expr()?;
            left = Expr::Union(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_path_expr(&mut self) -> Result<Expr, CfmlError> {
        if matches!(self.peek(), Tok::Slash | Tok::DoubleSlash) {
            return Ok(Expr::Path(self.parse_location_path()?));
        }

        if self.starts_primary() {
            let mut primary = self.parse_primary()?;
            let mut preds = Vec::new();
            while matches!(self.peek(), Tok::LBrack) {
                self.bump();
                preds.push(self.parse_or()?);
                self.expect(&Tok::RBrack)?;
            }
            if !preds.is_empty() {
                primary = Expr::Filter(Box::new(primary), preds);
            }
            if matches!(self.peek(), Tok::Slash | Tok::DoubleSlash) {
                let rest = self.parse_relative_after_filter()?;
                return Ok(Expr::PathFrom(Box::new(primary), rest));
            }
            return Ok(primary);
        }

        Ok(Expr::Path(self.parse_location_path()?))
    }

    fn starts_primary(&self) -> bool {
        match self.peek() {
            Tok::LParen | Tok::String(_) | Tok::Number(_) => true,
            Tok::Name(n) => {
                matches!(self.peek_at(1), Tok::LParen) && !is_axis_then_colon_colon(self, n)
            }
            _ => false,
        }
    }

    fn parse_location_path(&mut self) -> Result<PathExpr, CfmlError> {
        let mut absolute = false;
        let mut leading_descendant = false;
        match self.peek() {
            Tok::DoubleSlash => {
                self.bump();
                absolute = true;
                leading_descendant = true;
            }
            Tok::Slash => {
                self.bump();
                absolute = true;
                if self.at_path_end() {
                    return Ok(PathExpr {
                        absolute: true,
                        leading_descendant: false,
                        steps: Vec::new(),
                    });
                }
            }
            _ => {}
        }
        let steps = self.parse_relative_steps()?;
        if steps.is_empty() && !absolute {
            return Err(CfmlError::runtime(
                "XmlSearch: invalid XPath: expected location step".to_string(),
            ));
        }
        Ok(PathExpr {
            absolute,
            leading_descendant,
            steps,
        })
    }

    fn parse_relative_after_filter(&mut self) -> Result<PathExpr, CfmlError> {
        let mut leading_descendant = false;
        let mut steps = Vec::new();
        match self.peek() {
            Tok::DoubleSlash => {
                self.bump();
                leading_descendant = true;
                steps.push(self.parse_step()?);
                steps.extend(self.parse_more_steps()?);
            }
            Tok::Slash => {
                self.bump();
                steps.push(self.parse_step()?);
                steps.extend(self.parse_more_steps()?);
            }
            _ => {
                return Err(CfmlError::runtime(
                    "XmlSearch: invalid XPath: expected '/' after filter".to_string(),
                ));
            }
        }
        Ok(PathExpr {
            absolute: false,
            leading_descendant,
            steps,
        })
    }

    fn at_path_end(&self) -> bool {
        match self.peek() {
            Tok::Eof | Tok::Pipe | Tok::RParen | Tok::RBrack | Tok::Comma => true,
            Tok::Name(n) if matches!(n.as_str(), "or" | "and" | "div" | "mod") => true,
            Tok::Eq | Tok::Ne | Tok::Lt | Tok::Le | Tok::Gt | Tok::Ge | Tok::Plus | Tok::Minus => {
                true
            }
            _ => false,
        }
    }

    fn parse_relative_steps(&mut self) -> Result<Vec<Step>, CfmlError> {
        let mut steps = Vec::new();
        if self.at_step_start() {
            steps.push(self.parse_step()?);
            steps.extend(self.parse_more_steps()?);
        }
        Ok(steps)
    }

    fn parse_more_steps(&mut self) -> Result<Vec<Step>, CfmlError> {
        let mut steps = Vec::new();
        loop {
            match self.peek() {
                Tok::Slash => {
                    self.bump();
                    steps.push(self.parse_step()?);
                }
                Tok::DoubleSlash => {
                    self.bump();
                    steps.push(Step {
                        axis: Axis::DescendantOrSelf,
                        test: NodeTest::Node,
                        predicates: Vec::new(),
                    });
                    steps.push(self.parse_step()?);
                }
                _ => break,
            }
        }
        Ok(steps)
    }

    fn at_step_start(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Dot | Tok::DotDot | Tok::At | Tok::Star | Tok::Name(_)
        )
    }

    fn parse_step(&mut self) -> Result<Step, CfmlError> {
        match self.peek() {
            Tok::Dot => {
                self.bump();
                Ok(Step {
                    axis: Axis::Oneself,
                    test: NodeTest::Node,
                    predicates: self.parse_predicates()?,
                })
            }
            Tok::DotDot => {
                self.bump();
                Ok(Step {
                    axis: Axis::Parent,
                    test: NodeTest::Node,
                    predicates: self.parse_predicates()?,
                })
            }
            _ => {
                let (axis, test) = self.parse_axis_node_test()?;
                Ok(Step {
                    axis,
                    test,
                    predicates: self.parse_predicates()?,
                })
            }
        }
    }

    fn parse_axis_node_test(&mut self) -> Result<(Axis, NodeTest), CfmlError> {
        if matches!(self.peek(), Tok::At) {
            self.bump();
            let test = self.parse_node_test()?;
            return Ok((Axis::Attribute, test));
        }

        if let Tok::Name(name) = self.peek().clone() {
            if matches!(self.peek_at(1), Tok::ColonColon) {
                self.bump();
                self.bump();
                let axis = match name.as_str() {
                    "child" => Axis::Child,
                    "descendant" => Axis::Descendant,
                    "descendant-or-self" => Axis::DescendantOrSelf,
                    "parent" => Axis::Parent,
                    "self" => Axis::Oneself,
                    "attribute" => Axis::Attribute,
                    other => {
                        return Err(CfmlError::runtime(format!(
                            "XmlSearch: unsupported XPath axis '{}'",
                            other
                        )));
                    }
                };
                let test = self.parse_node_test()?;
                return Ok((axis, test));
            }
        }

        Ok((Axis::Child, self.parse_node_test()?))
    }

    fn parse_node_test(&mut self) -> Result<NodeTest, CfmlError> {
        match self.peek().clone() {
            Tok::Star => {
                self.bump();
                Ok(NodeTest::Wildcard)
            }
            Tok::Name(n) => {
                if is_node_type_name(&n) && matches!(self.peek_at(1), Tok::LParen) {
                    self.bump();
                    self.expect(&Tok::LParen)?;
                    self.expect(&Tok::RParen)?;
                    return match n.as_str() {
                        "text" => Ok(NodeTest::Text),
                        "node" => Ok(NodeTest::Node),
                        other => Err(CfmlError::runtime(format!(
                            "XmlSearch: unsupported XPath node type '{}()'",
                            other
                        ))),
                    };
                }
                self.bump();
                parse_name_test_str(&n)
            }
            other => Err(CfmlError::runtime(format!(
                "XmlSearch: invalid XPath: expected name test, got {:?}",
                other
            ))),
        }
    }

    fn parse_predicates(&mut self) -> Result<Vec<Expr>, CfmlError> {
        let mut preds = Vec::new();
        while matches!(self.peek(), Tok::LBrack) {
            self.bump();
            preds.push(self.parse_or()?);
            self.expect(&Tok::RBrack)?;
        }
        Ok(preds)
    }

    fn parse_primary(&mut self) -> Result<Expr, CfmlError> {
        match self.peek().clone() {
            Tok::LParen => {
                self.bump();
                let e = self.parse_or()?;
                self.expect(&Tok::RParen)?;
                Ok(e)
            }
            Tok::String(s) => {
                self.bump();
                Ok(Expr::Literal(s))
            }
            Tok::Number(n) => {
                self.bump();
                Ok(Expr::Number(n))
            }
            Tok::Name(n) => {
                self.bump();
                self.expect(&Tok::LParen)?;
                let mut args = Vec::new();
                if !matches!(self.peek(), Tok::RParen) {
                    args.push(self.parse_or()?);
                    while matches!(self.peek(), Tok::Comma) {
                        self.bump();
                        args.push(self.parse_or()?);
                    }
                }
                self.expect(&Tok::RParen)?;
                // `text()` as a primary is treated as child::text() relative path
                // (common CFML / Lucee idiom inside predicates and functions).
                if n.eq_ignore_ascii_case("text") && args.is_empty() {
                    return Ok(Expr::Path(PathExpr {
                        absolute: false,
                        leading_descendant: false,
                        steps: vec![Step {
                            axis: Axis::Child,
                            test: NodeTest::Text,
                            predicates: Vec::new(),
                        }],
                    }));
                }
                Ok(Expr::Function(n.to_ascii_lowercase(), args))
            }
            other => Err(CfmlError::runtime(format!(
                "XmlSearch: invalid XPath: unexpected token {:?}",
                other
            ))),
        }
    }
}

fn is_axis_then_colon_colon(p: &Parser, _n: &str) -> bool {
    matches!(p.peek_at(1), Tok::ColonColon)
}

fn is_node_type_name(n: &str) -> bool {
    matches!(n, "text" | "node" | "comment" | "processing-instruction")
}

fn parse_name_test_str(n: &str) -> Result<NodeTest, CfmlError> {
    if n == "*" {
        return Ok(NodeTest::Wildcard);
    }
    if let Some((prefix, local)) = n.split_once(':') {
        if local == "*" {
            Ok(NodeTest::PrefixWildcard {
                prefix: prefix.to_string(),
            })
        } else {
            Ok(NodeTest::Name {
                prefix: Some(prefix.to_string()),
                local: local.to_string(),
            })
        }
    } else {
        Ok(NodeTest::Name {
            prefix: None,
            local: n.to_string(),
        })
    }
}

fn parse_xpath(expr: &str) -> Result<Expr, CfmlError> {
    Parser::new(expr)?.parse()
}

// ---------------------------------------------------------------------------
// Evaluator
// ---------------------------------------------------------------------------

struct EvalContext<'a> {
    dom: &'a Dom,
    position: usize,
    size: usize,
    ns: HashMap<String, String>,
}

fn eval_expr(
    expr: &Expr,
    context_nodes: &[usize],
    ctx: &mut EvalContext<'_>,
) -> Result<XPathValue, CfmlError> {
    match expr {
        Expr::Or(a, b) => {
            let left = eval_expr(a, context_nodes, ctx)?;
            if left.to_boolean() {
                Ok(XPathValue::Bool(true))
            } else {
                Ok(XPathValue::Bool(
                    eval_expr(b, context_nodes, ctx)?.to_boolean(),
                ))
            }
        }
        Expr::And(a, b) => {
            let left = eval_expr(a, context_nodes, ctx)?;
            if !left.to_boolean() {
                Ok(XPathValue::Bool(false))
            } else {
                Ok(XPathValue::Bool(
                    eval_expr(b, context_nodes, ctx)?.to_boolean(),
                ))
            }
        }
        Expr::Eq(a, b) => cmp_eq(a, b, context_nodes, ctx, true),
        Expr::Ne(a, b) => cmp_eq(a, b, context_nodes, ctx, false),
        Expr::Lt(a, b) => cmp_rel(a, b, context_nodes, ctx, |x, y| x < y),
        Expr::Le(a, b) => cmp_rel(a, b, context_nodes, ctx, |x, y| x <= y),
        Expr::Gt(a, b) => cmp_rel(a, b, context_nodes, ctx, |x, y| x > y),
        Expr::Ge(a, b) => cmp_rel(a, b, context_nodes, ctx, |x, y| x >= y),
        Expr::Add(a, b) => {
            let x = eval_expr(a, context_nodes, ctx)?.to_number(ctx.dom);
            let y = eval_expr(b, context_nodes, ctx)?.to_number(ctx.dom);
            Ok(XPathValue::Number(x + y))
        }
        Expr::Sub(a, b) => {
            let x = eval_expr(a, context_nodes, ctx)?.to_number(ctx.dom);
            let y = eval_expr(b, context_nodes, ctx)?.to_number(ctx.dom);
            Ok(XPathValue::Number(x - y))
        }
        Expr::Mul(a, b) => {
            let x = eval_expr(a, context_nodes, ctx)?.to_number(ctx.dom);
            let y = eval_expr(b, context_nodes, ctx)?.to_number(ctx.dom);
            Ok(XPathValue::Number(x * y))
        }
        Expr::Div(a, b) => {
            let x = eval_expr(a, context_nodes, ctx)?.to_number(ctx.dom);
            let y = eval_expr(b, context_nodes, ctx)?.to_number(ctx.dom);
            Ok(XPathValue::Number(x / y))
        }
        Expr::Mod(a, b) => {
            let x = eval_expr(a, context_nodes, ctx)?.to_number(ctx.dom);
            let y = eval_expr(b, context_nodes, ctx)?.to_number(ctx.dom);
            Ok(XPathValue::Number(x % y))
        }
        Expr::Neg(a) => {
            let x = eval_expr(a, context_nodes, ctx)?.to_number(ctx.dom);
            Ok(XPathValue::Number(-x))
        }
        Expr::Union(a, b) => {
            let mut left = match eval_expr(a, context_nodes, ctx)? {
                XPathValue::Nodes(n) => n,
                other => {
                    return Err(CfmlError::runtime(format!(
                        "XmlSearch: union '|' requires node-sets, got {:?}",
                        other
                    )));
                }
            };
            let right = match eval_expr(b, context_nodes, ctx)? {
                XPathValue::Nodes(n) => n,
                other => {
                    return Err(CfmlError::runtime(format!(
                        "XmlSearch: union '|' requires node-sets, got {:?}",
                        other
                    )));
                }
            };
            left.extend(right);
            unique_doc_order(ctx.dom, &mut left);
            Ok(XPathValue::Nodes(left))
        }
        Expr::Path(path) => eval_path(path, context_nodes, ctx),
        Expr::PathFrom(filter, path) => {
            let nodes = match eval_expr(filter, context_nodes, ctx)? {
                XPathValue::Nodes(n) => n,
                other => {
                    return Err(CfmlError::runtime(format!(
                        "XmlSearch: path after filter requires a node-set, got {:?}",
                        other
                    )));
                }
            };
            eval_path_relative(path, &nodes, ctx)
        }
        Expr::Filter(primary, preds) => {
            let nodes = match eval_expr(primary, context_nodes, ctx)? {
                XPathValue::Nodes(n) => n,
                other => {
                    return Err(CfmlError::runtime(format!(
                        "XmlSearch: predicates require a node-set, got {:?}",
                        other
                    )));
                }
            };
            Ok(XPathValue::Nodes(apply_predicates(&nodes, preds, ctx)?))
        }
        Expr::Literal(s) => Ok(XPathValue::String(s.clone())),
        Expr::Number(n) => Ok(XPathValue::Number(*n)),
        Expr::Function(name, args) => eval_function(name, args, context_nodes, ctx),
    }
}

fn cmp_eq(
    a: &Expr,
    b: &Expr,
    context_nodes: &[usize],
    ctx: &mut EvalContext<'_>,
    want_eq: bool,
) -> Result<XPathValue, CfmlError> {
    let left = eval_expr(a, context_nodes, ctx)?;
    let right = eval_expr(b, context_nodes, ctx)?;
    let result = match (&left, &right) {
        (XPathValue::Nodes(ln), XPathValue::Nodes(rn)) => {
            let mut ok = false;
            'outer: for &li in ln {
                let ls = ctx.dom.string_value(li);
                for &ri in rn {
                    if ls == ctx.dom.string_value(ri) {
                        ok = true;
                        break 'outer;
                    }
                }
            }
            ok
        }
        (XPathValue::Nodes(ln), XPathValue::Number(n))
        | (XPathValue::Number(n), XPathValue::Nodes(ln)) => ln
            .iter()
            .any(|&i| parse_xpath_number(&ctx.dom.string_value(i)) == *n),
        (XPathValue::Nodes(ln), XPathValue::String(s))
        | (XPathValue::String(s), XPathValue::Nodes(ln)) => {
            ln.iter().any(|&i| ctx.dom.string_value(i) == *s)
        }
        (XPathValue::Nodes(ln), XPathValue::Bool(bv))
        | (XPathValue::Bool(bv), XPathValue::Nodes(ln)) => {
            XPathValue::Nodes(ln.clone()).to_boolean() == *bv
        }
        (XPathValue::Bool(_), _) | (_, XPathValue::Bool(_)) => {
            left.to_boolean() == right.to_boolean()
        }
        (XPathValue::Number(_), _) | (_, XPathValue::Number(_)) => {
            let ln = left.to_number(ctx.dom);
            let rn = right.to_number(ctx.dom);
            if ln.is_nan() || rn.is_nan() {
                false
            } else {
                ln == rn
            }
        }
        _ => left.to_string_value(ctx.dom) == right.to_string_value(ctx.dom),
    };
    Ok(XPathValue::Bool(if want_eq { result } else { !result }))
}

fn cmp_rel(
    a: &Expr,
    b: &Expr,
    context_nodes: &[usize],
    ctx: &mut EvalContext<'_>,
    op: impl Fn(f64, f64) -> bool,
) -> Result<XPathValue, CfmlError> {
    let left = eval_expr(a, context_nodes, ctx)?;
    let right = eval_expr(b, context_nodes, ctx)?;
    // XPath 1.0: convert both to numbers (node-set comparison expands).
    let result = match (&left, &right) {
        (XPathValue::Nodes(ln), XPathValue::Nodes(rn)) => {
            let mut ok = false;
            'outer: for &li in ln {
                let lv = parse_xpath_number(&ctx.dom.string_value(li));
                for &ri in rn {
                    let rv = parse_xpath_number(&ctx.dom.string_value(ri));
                    if op(lv, rv) {
                        ok = true;
                        break 'outer;
                    }
                }
            }
            ok
        }
        (XPathValue::Nodes(ln), other) => {
            let rv = other.to_number(ctx.dom);
            ln.iter()
                .any(|&i| op(parse_xpath_number(&ctx.dom.string_value(i)), rv))
        }
        (other, XPathValue::Nodes(rn)) => {
            let lv = other.to_number(ctx.dom);
            rn.iter()
                .any(|&i| op(lv, parse_xpath_number(&ctx.dom.string_value(i))))
        }
        _ => {
            let lv = left.to_number(ctx.dom);
            let rv = right.to_number(ctx.dom);
            !lv.is_nan() && !rv.is_nan() && op(lv, rv)
        }
    };
    Ok(XPathValue::Bool(result))
}

fn eval_path(
    path: &PathExpr,
    context_nodes: &[usize],
    ctx: &mut EvalContext<'_>,
) -> Result<XPathValue, CfmlError> {
    let start = if path.absolute {
        // Absolute paths start at the document node(s).
        let mut docs = Vec::new();
        for &r in &ctx.dom.roots {
            let doc = find_document(ctx.dom, r);
            docs.push(doc);
        }
        unique_doc_order(ctx.dom, &mut docs);
        docs
    } else {
        context_nodes.to_vec()
    };
    eval_path_from(path, &start, ctx)
}

fn eval_path_relative(
    path: &PathExpr,
    context_nodes: &[usize],
    ctx: &mut EvalContext<'_>,
) -> Result<XPathValue, CfmlError> {
    eval_path_from(path, context_nodes, ctx)
}

fn eval_path_from(
    path: &PathExpr,
    start: &[usize],
    ctx: &mut EvalContext<'_>,
) -> Result<XPathValue, CfmlError> {
    let mut nodes = start.to_vec();
    if path.leading_descendant {
        // `//` ≡ descendant-or-self::node() before the first step's child axis
        // Abbreviated `//foo` = /descendant-or-self::node()/child::foo
        let mut expanded = Vec::new();
        for &n in &nodes {
            collect_descendant_or_self(ctx.dom, n, &mut expanded);
        }
        unique_doc_order(ctx.dom, &mut expanded);
        nodes = expanded;
    }

    if path.steps.is_empty() {
        return Ok(XPathValue::Nodes(nodes));
    }

    // When leading_descendant is set, the first step uses the child axis against
    // the expanded set (standard `//` expansion). Steps are applied as written.
    for step in &path.steps {
        nodes = eval_step(step, &nodes, ctx)?;
    }
    Ok(XPathValue::Nodes(nodes))
}

fn find_document(dom: &Dom, idx: usize) -> usize {
    let mut cur = idx;
    loop {
        if dom.nodes[cur].kind == NodeKind::Document {
            return cur;
        }
        match dom.nodes[cur].parent {
            Some(p) => cur = p,
            None => return cur,
        }
    }
}

fn eval_step(
    step: &Step,
    context_nodes: &[usize],
    ctx: &mut EvalContext<'_>,
) -> Result<Vec<usize>, CfmlError> {
    // XPath 1.0: for each context node, build the axis node-set, apply the
    // node test, then filter by predicates with positions relative to THAT
    // node-set — then union the survivors (document order, unique).
    let mut results = Vec::new();
    for &ctx_node in context_nodes {
        let mut axis_nodes = Vec::new();
        match step.axis {
            Axis::Child => {
                axis_nodes.extend_from_slice(&ctx.dom.nodes[ctx_node].children);
            }
            Axis::Descendant => {
                collect_descendants(ctx.dom, ctx_node, &mut axis_nodes);
            }
            Axis::DescendantOrSelf => {
                collect_descendant_or_self(ctx.dom, ctx_node, &mut axis_nodes);
            }
            Axis::Parent => {
                if let Some(p) = ctx.dom.nodes[ctx_node].parent {
                    axis_nodes.push(p);
                }
            }
            Axis::Oneself => {
                axis_nodes.push(ctx_node);
            }
            Axis::Attribute => {
                axis_nodes.extend_from_slice(&ctx.dom.nodes[ctx_node].attributes);
            }
        }

        let mut matched = Vec::new();
        for n in axis_nodes {
            if node_test_matches(ctx, n, &step.test, step.axis == Axis::Attribute) {
                matched.push(n);
            }
        }
        // Axis order for forward axes is document order; parent/ancestor are
        // reverse — we only expose parent, whose node-set size is ≤ 1.
        unique_doc_order(ctx.dom, &mut matched);
        let filtered = apply_predicates(&matched, &step.predicates, ctx)?;
        results.extend(filtered);
    }
    unique_doc_order(ctx.dom, &mut results);
    Ok(results)
}

fn collect_descendants(dom: &Dom, idx: usize, out: &mut Vec<usize>) {
    for &c in &dom.nodes[idx].children {
        out.push(c);
        collect_descendants(dom, c, out);
    }
}

fn collect_descendant_or_self(dom: &Dom, idx: usize, out: &mut Vec<usize>) {
    out.push(idx);
    collect_descendants(dom, idx, out);
}

fn node_test_matches(
    ctx: &EvalContext<'_>,
    idx: usize,
    test: &NodeTest,
    on_attribute_axis: bool,
) -> bool {
    let node = &ctx.dom.nodes[idx];
    match test {
        NodeTest::Node => true,
        NodeTest::Text => node.kind == NodeKind::Text,
        NodeTest::Wildcard => {
            if on_attribute_axis {
                node.kind == NodeKind::Attribute
            } else {
                node.kind == NodeKind::Element
            }
        }
        NodeTest::PrefixWildcard { prefix } => {
            let kind_ok = if on_attribute_axis {
                node.kind == NodeKind::Attribute
            } else {
                node.kind == NodeKind::Element
            };
            if !kind_ok {
                return false;
            }
            let uri = ctx.ns.get(prefix).cloned().unwrap_or_default();
            !uri.is_empty() && node.namespace_uri == uri
        }
        NodeTest::Name { prefix, local } => {
            let kind_ok = if on_attribute_axis {
                node.kind == NodeKind::Attribute
            } else {
                node.kind == NodeKind::Element
            };
            if !kind_ok {
                return false;
            }
            match prefix {
                Some(p) => {
                    let uri = ctx.ns.get(p).cloned().unwrap_or_default();
                    if !uri.is_empty() {
                        node.local_name == *local && node.namespace_uri == uri
                    } else {
                        // Prefix unknown in the XPath context: match the
                        // literal QName as stored on the node (Lucee does this
                        // for document-declared prefixes that somehow miss the
                        // context map).
                        node.qname == format!("{}:{}", p, local)
                    }
                }
                None => {
                    // Unprefixed name test (XPath 1.0): empty namespace + local
                    // name. Also accept a literal qname match so `//item` keeps
                    // working on plain CFML documents.
                    (node.namespace_uri.is_empty() && node.local_name == *local)
                        || node.qname == *local
                }
            }
        }
    }
}

fn apply_predicates(
    nodes: &[usize],
    preds: &[Expr],
    ctx: &mut EvalContext<'_>,
) -> Result<Vec<usize>, CfmlError> {
    let mut current = nodes.to_vec();
    for pred in preds {
        let size = current.len();
        let mut next = Vec::new();
        for (i, &n) in current.iter().enumerate() {
            ctx.position = i + 1;
            ctx.size = size;
            let v = eval_expr(pred, &[n], ctx)?;
            let keep = match v {
                XPathValue::Number(num) => (ctx.position as f64) == num,
                other => other.to_boolean(),
            };
            if keep {
                next.push(n);
            }
        }
        current = next;
    }
    Ok(current)
}

fn eval_function(
    name: &str,
    args: &[Expr],
    context_nodes: &[usize],
    ctx: &mut EvalContext<'_>,
) -> Result<XPathValue, CfmlError> {
    let ctx_node = context_nodes.first().copied();

    match name {
        "position" => {
            require_arity(name, args, 0)?;
            Ok(XPathValue::Number(ctx.position as f64))
        }
        "last" => {
            require_arity(name, args, 0)?;
            Ok(XPathValue::Number(ctx.size as f64))
        }
        "count" => {
            require_arity(name, args, 1)?;
            match eval_expr(&args[0], context_nodes, ctx)? {
                XPathValue::Nodes(ns) => Ok(XPathValue::Number(ns.len() as f64)),
                _ => Err(CfmlError::runtime(
                    "XmlSearch: count() expects a node-set".to_string(),
                )),
            }
        }
        "local-name" => {
            let idx = node_arg_or_context(name, args, context_nodes, ctx, ctx_node)?;
            Ok(XPathValue::String(
                idx.map(|i| ctx.dom.nodes[i].local_name.clone())
                    .unwrap_or_default(),
            ))
        }
        "name" => {
            let idx = node_arg_or_context(name, args, context_nodes, ctx, ctx_node)?;
            Ok(XPathValue::String(
                idx.map(|i| ctx.dom.nodes[i].qname.clone())
                    .unwrap_or_default(),
            ))
        }
        "namespace-uri" => {
            let idx = node_arg_or_context(name, args, context_nodes, ctx, ctx_node)?;
            Ok(XPathValue::String(
                idx.map(|i| ctx.dom.nodes[i].namespace_uri.clone())
                    .unwrap_or_default(),
            ))
        }
        "string" => {
            if args.is_empty() {
                let s = ctx_node
                    .map(|i| ctx.dom.string_value(i))
                    .unwrap_or_default();
                Ok(XPathValue::String(s))
            } else {
                require_arity(name, args, 1)?;
                Ok(XPathValue::String(
                    eval_expr(&args[0], context_nodes, ctx)?.to_string_value(ctx.dom),
                ))
            }
        }
        "number" => {
            if args.is_empty() {
                let s = ctx_node
                    .map(|i| ctx.dom.string_value(i))
                    .unwrap_or_default();
                Ok(XPathValue::Number(parse_xpath_number(&s)))
            } else {
                require_arity(name, args, 1)?;
                Ok(XPathValue::Number(
                    eval_expr(&args[0], context_nodes, ctx)?.to_number(ctx.dom),
                ))
            }
        }
        "boolean" => {
            require_arity(name, args, 1)?;
            Ok(XPathValue::Bool(
                eval_expr(&args[0], context_nodes, ctx)?.to_boolean(),
            ))
        }
        "not" => {
            require_arity(name, args, 1)?;
            Ok(XPathValue::Bool(
                !eval_expr(&args[0], context_nodes, ctx)?.to_boolean(),
            ))
        }
        "contains" => {
            require_arity(name, args, 2)?;
            let a = eval_expr(&args[0], context_nodes, ctx)?.to_string_value(ctx.dom);
            let b = eval_expr(&args[1], context_nodes, ctx)?.to_string_value(ctx.dom);
            Ok(XPathValue::Bool(a.contains(&b)))
        }
        "starts-with" => {
            require_arity(name, args, 2)?;
            let a = eval_expr(&args[0], context_nodes, ctx)?.to_string_value(ctx.dom);
            let b = eval_expr(&args[1], context_nodes, ctx)?.to_string_value(ctx.dom);
            Ok(XPathValue::Bool(a.starts_with(&b)))
        }
        "normalize-space" => {
            let s = if args.is_empty() {
                ctx_node
                    .map(|i| ctx.dom.string_value(i))
                    .unwrap_or_default()
            } else {
                require_arity(name, args, 1)?;
                eval_expr(&args[0], context_nodes, ctx)?.to_string_value(ctx.dom)
            };
            Ok(XPathValue::String(normalize_space(&s)))
        }
        "string-length" => {
            let s = if args.is_empty() {
                ctx_node
                    .map(|i| ctx.dom.string_value(i))
                    .unwrap_or_default()
            } else {
                require_arity(name, args, 1)?;
                eval_expr(&args[0], context_nodes, ctx)?.to_string_value(ctx.dom)
            };
            Ok(XPathValue::Number(s.chars().count() as f64))
        }
        "true" => {
            require_arity(name, args, 0)?;
            Ok(XPathValue::Bool(true))
        }
        "false" => {
            require_arity(name, args, 0)?;
            Ok(XPathValue::Bool(false))
        }
        other => Err(CfmlError::runtime(format!(
            "XmlSearch: unsupported or unknown XPath function '{}()'",
            other
        ))),
    }
}

fn require_arity(name: &str, args: &[Expr], n: usize) -> Result<(), CfmlError> {
    if args.len() == n {
        Ok(())
    } else {
        Err(CfmlError::runtime(format!(
            "XmlSearch: {}() expects {} argument(s), got {}",
            name,
            n,
            args.len()
        )))
    }
}

fn node_arg_or_context(
    name: &str,
    args: &[Expr],
    context_nodes: &[usize],
    ctx: &mut EvalContext<'_>,
    ctx_node: Option<usize>,
) -> Result<Option<usize>, CfmlError> {
    if args.is_empty() {
        Ok(ctx_node)
    } else if args.len() == 1 {
        match eval_expr(&args[0], context_nodes, ctx)? {
            XPathValue::Nodes(ns) => Ok(ns.first().copied()),
            _ => Err(CfmlError::runtime(format!(
                "XmlSearch: {}() argument must be a node-set",
                name
            ))),
        }
    } else {
        Err(CfmlError::runtime(format!(
            "XmlSearch: {}() expects 0 or 1 argument(s), got {}",
            name,
            args.len()
        )))
    }
}

fn normalize_space(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use cfml_common::dynamic::ValueMap;

    /// Minimal element builder mirroring xmlParse's shape.
    fn elem(name: &str, attrs: &[(&str, &str)], text: &str, children: Vec<CfmlValue>) -> CfmlValue {
        let mut m = ValueMap::default();
        m.insert("xmlName".to_string(), CfmlValue::string(name.to_string()));
        m.insert(
            "xmlType".to_string(),
            CfmlValue::string("ELEMENT".to_string()),
        );
        m.insert("xmlText".to_string(), CfmlValue::string(text.to_string()));
        let mut am = ValueMap::default();
        for (k, v) in attrs {
            am.insert((*k).to_string(), CfmlValue::string((*v).to_string()));
        }
        m.insert("xmlAttributes".to_string(), CfmlValue::strukt(am));
        m.insert("xmlChildren".to_string(), CfmlValue::array(children));
        CfmlValue::strukt(m)
    }

    fn doc(root: CfmlValue) -> CfmlValue {
        let mut m = ValueMap::default();
        m.insert("xmlRoot".to_string(), root.clone());
        m.insert(
            "xmlType".to_string(),
            CfmlValue::string("DOCUMENT".to_string()),
        );
        if let CfmlValue::Struct(ref s) = root {
            if let Some(name) = s.get("xmlName") {
                m.insert(name.as_string(), root);
            }
        }
        CfmlValue::strukt(m)
    }

    fn ooxml_doc() -> CfmlValue {
        let t1 = elem("w:t", &[], "one", vec![]);
        let t2 = elem("w:t", &[], "two", vec![]);
        let r1 = elem("w:r", &[], "", vec![t1]);
        let r2 = elem("w:r", &[], "", vec![t2]);
        let p1 = elem("w:p", &[], "", vec![r1]);
        let p2 = elem("w:p", &[], "", vec![r2]);
        let body = elem("w:body", &[], "", vec![p1, p2]);
        let root = elem(
            "w:document",
            &[(
                "xmlns:w",
                "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
            )],
            "",
            vec![body],
        );
        doc(root)
    }

    fn array_len(v: &CfmlValue) -> usize {
        match v {
            CfmlValue::Array(a) => a.len(),
            _ => panic!("expected array, got {:?}", v),
        }
    }

    #[test]
    fn issue_548_prefixed_and_local_name() {
        let d = ooxml_doc();
        assert_eq!(array_len(&xml_search(&d, "//w:t").unwrap()), 2);
        assert_eq!(
            array_len(&xml_search(&d, "//*[local-name()='t']").unwrap()),
            2
        );
        assert_eq!(array_len(&xml_search(&d, "//*[name()='w:t']").unwrap()), 2);
    }

    #[test]
    fn positional_predicate_and_path() {
        let d = ooxml_doc();
        let r = xml_search(&d, "//w:p[2]/w:r/w:t").unwrap();
        assert_eq!(array_len(&r), 1);
        if let CfmlValue::Array(a) = r {
            let node = a.get(0).unwrap();
            if let CfmlValue::Struct(s) = node {
                assert_eq!(s.get("xmlText").unwrap().as_string(), "two");
            }
        }
    }

    #[test]
    fn attributes_and_count() {
        let item = elem("item", &[("id", "x")], "hi", vec![]);
        let root = elem(
            "root",
            &[],
            "",
            vec![item.clone(), elem("item", &[("id", "y")], "", vec![])],
        );
        let d = doc(root);
        assert_eq!(array_len(&xml_search(&d, "//@id").unwrap()), 2);
        match xml_search(&d, "count(//item)").unwrap() {
            CfmlValue::Int(n) => assert_eq!(n, 2),
            other => panic!("expected int, got {:?}", other),
        }
    }

    #[test]
    fn union_parent_predicates() {
        let d = ooxml_doc();
        assert_eq!(array_len(&xml_search(&d, "//w:t | //w:p").unwrap()), 4);
        let r = xml_search(&d, "//w:t[1]/..").unwrap();
        // Each <w:r> has one <w:t>, so [1] keeps both; parent axis yields both <w:r>s.
        assert_eq!(array_len(&r), 2);
        if let CfmlValue::Array(a) = &r {
            let node = a.get(0).unwrap();
            if let CfmlValue::Struct(s) = node {
                assert_eq!(s.get("xmlName").unwrap().as_string(), "w:r");
            }
        }
        assert_eq!(
            array_len(
                &xml_search(
                    &d,
                    "//w:t[contains(text(),'o') and starts-with(.,'t') or .='one']"
                )
                .unwrap()
            ),
            2
        );
        assert_eq!(
            array_len(&xml_search(&d, "//w:t[not(contains(.,'z'))]").unwrap()),
            2
        );
    }

    #[test]
    fn invalid_xpath_errors() {
        let d = ooxml_doc();
        let err = xml_search(&d, "//*[local-name(").unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("XmlSearch") && msg.contains("invalid"),
            "msg={}",
            msg
        );
        let err = xml_search(&d, "//foo[following::bar]").unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("unsupported") || msg.contains("invalid"),
            "msg={}",
            msg
        );
    }
}
