//! Options shared by `markdown()`, `htmlToMarkdown()`, `MarkdownDocument()`
//! and the object's renderers. App-wide defaults come from the `markdown`
//! block of `.cfconfig.json` (see [`set_defaults`]); a per-call struct
//! overrides them key by key.

use std::sync::RwLock;

use cfml_common::dynamic::CfmlValue;
use cfml_common::vm::{CfmlError, CfmlErrorType};

pub fn md_err(msg: impl Into<String>) -> CfmlError {
    CfmlError::new(msg.into(), CfmlErrorType::Custom("Markdown".to_string()))
}

#[derive(Clone, Debug, PartialEq)]
pub struct MdOptions {
    pub gfm: bool,
    pub footnotes: bool,
    pub front_matter: bool,
    /// Render raw HTML and every URL scheme. Off: raw HTML is dropped and
    /// `javascript:` / `vbscript:` / `file:` / non-image `data:` URLs are
    /// blanked (comrak's safe mode).
    pub unsafe_html: bool,
    /// With `unsafe_html` off, show raw HTML as escaped text instead of
    /// dropping it.
    pub escape_html: bool,
    /// Give headings ids and anchor links.
    pub anchors: bool,
    pub hard_breaks: bool,
    pub table_class: String,
    /// Strip the indentation every non-blank line shares before parsing, so
    /// markdown written indented inside a template or a multi-line string is
    /// not read as a code block.
    pub dedent: bool,
    /// `toMarkdown()` line width; 0 = do not wrap.
    pub width: usize,
    /// `toMarkdown()` bullet: `-`, `*` or `+`.
    pub list_marker: char,
}

impl Default for MdOptions {
    fn default() -> Self {
        MdOptions {
            gfm: true,
            footnotes: false,
            front_matter: true,
            unsafe_html: false,
            escape_html: false,
            anchors: false,
            hard_breaks: false,
            table_class: String::new(),
            dedent: false,
            width: 0,
            list_marker: '-',
        }
    }
}

static DEFAULTS: RwLock<Option<MdOptions>> = RwLock::new(None);

/// Install the app-wide defaults from `.cfconfig.json`'s `markdown` block.
pub fn set_defaults(cfg: &cfml_config::schema::MarkdownCfg) {
    let o = MdOptions {
        gfm: cfg.gfm,
        footnotes: cfg.footnotes,
        front_matter: cfg.front_matter,
        unsafe_html: cfg.unsafe_html,
        escape_html: cfg.escape_html,
        anchors: cfg.anchors,
        hard_breaks: cfg.hard_breaks,
        table_class: cfg.table_class.clone(),
        ..MdOptions::default()
    };
    *DEFAULTS.write().unwrap_or_else(|e| e.into_inner()) = Some(o);
}

pub fn defaults() -> MdOptions {
    DEFAULTS.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

const KEYS: &str = "gfm, footnotes, frontMatter, unsafe, escapeHtml, anchors, hardBreaks, \
                    tableClass, dedent, width, listMarker";

impl MdOptions {
    /// `base` overlaid with the keys of `value` (a struct, or null/absent).
    pub fn overlay(base: &MdOptions, value: Option<&CfmlValue>) -> Result<MdOptions, CfmlError> {
        let mut o = base.clone();
        let Some(v) = value else { return Ok(o) };
        match v {
            CfmlValue::Null => return Ok(o),
            CfmlValue::String(s) if s.trim().is_empty() => return Ok(o),
            CfmlValue::Struct(_) => {}
            other => {
                return Err(md_err(format!(
                    "Markdown options must be a struct, not {}. Valid keys: {}.",
                    other.type_name(),
                    KEYS
                )))
            }
        }
        let map = v.as_struct().unwrap_or_default();
        for (k, val) in map.iter() {
            if matches!(val, CfmlValue::Null) {
                continue;
            }
            match k.as_str().to_ascii_lowercase().as_str() {
                "gfm" => o.gfm = val.is_true(),
                "footnotes" => o.footnotes = val.is_true(),
                "frontmatter" => o.front_matter = val.is_true(),
                "unsafe" => o.unsafe_html = val.is_true(),
                "escapehtml" => o.escape_html = val.is_true(),
                "anchors" => o.anchors = val.is_true(),
                "hardbreaks" => o.hard_breaks = val.is_true(),
                "tableclass" => o.table_class = val.as_string(),
                "dedent" => o.dedent = val.is_true(),
                "width" => {
                    let s = val.as_string();
                    o.width = s.trim().parse::<f64>().ok().filter(|n| *n >= 0.0).map(|n| n as usize).ok_or_else(|| {
                        md_err(format!("Markdown option [width] must be a non-negative number, not [{}].", s))
                    })?;
                }
                "listmarker" => {
                    let s = val.as_string();
                    o.list_marker = match s.trim() {
                        "-" => '-',
                        "*" => '*',
                        "+" => '+',
                        other => {
                            return Err(md_err(format!(
                                "Markdown option [listMarker] must be -, * or +, not [{}].",
                                other
                            )))
                        }
                    };
                }
                other => {
                    return Err(md_err(format!(
                        "Unknown markdown option [{}]. Valid keys: {}.",
                        other, KEYS
                    )))
                }
            }
        }
        Ok(o)
    }

    pub fn comrak(&self) -> comrak::Options<'static> {
        let mut o = comrak::Options::default();
        o.extension.strikethrough = self.gfm;
        o.extension.table = self.gfm;
        o.extension.autolink = self.gfm;
        o.extension.tasklist = self.gfm;
        o.extension.footnotes = self.footnotes;
        if self.front_matter {
            o.extension.front_matter_delimiter = Some("---".to_string());
        }
        if self.anchors {
            o.extension.header_id_prefix = Some(String::new());
        }
        o.render.r#unsafe = self.unsafe_html;
        o.render.escape = self.escape_html && !self.unsafe_html;
        o.render.hardbreaks = self.hard_breaks;
        o.render.width = self.width;
        o.render.prefer_fenced = true;
        o.render.list_style = match self.list_marker {
            '*' => comrak::options::ListStyleType::Star,
            '+' => comrak::options::ListStyleType::Plus,
            _ => comrak::options::ListStyleType::Dash,
        };
        o
    }
}

/// Remove the indentation every non-blank line shares, and any blank lines
/// before the first non-blank one.
pub fn dedent(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let start = lines.iter().position(|l| !l.trim().is_empty()).unwrap_or(lines.len());
    let lines = &lines[start..];
    let mut prefix: Option<&str> = None;
    for l in lines.iter().filter(|l| !l.trim().is_empty()) {
        let ws_len = l.len() - l.trim_start_matches([' ', '\t']).len();
        let ws = &l[..ws_len];
        prefix = Some(match prefix {
            None => ws,
            Some(p) => {
                let common = p.bytes().zip(ws.bytes()).take_while(|(a, b)| a == b).count();
                &p[..common]
            }
        });
    }
    let cut = prefix.map(|p| p.len()).unwrap_or(0);
    let mut out = String::with_capacity(src.len());
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        if l.len() >= cut {
            out.push_str(&l[cut..]);
        } else {
            out.push_str(l.trim_start_matches([' ', '\t']));
        }
    }
    out
}
