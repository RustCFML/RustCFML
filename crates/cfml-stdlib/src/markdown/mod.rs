//! Built-in markdown: `markdown()`, `htmlToMarkdown()`, `MarkdownDocument()`,
//! `isMarkdownDocument()`, `markdownEscape()`, and the `<cfmarkdown>` tag.
//!
//! One parser (comrak) for everything, so `markdown( s )` and
//! `MarkdownDocument( s ).toHtml()` can never disagree. The document object
//! holds our own tree ([`tree`]) and converts to comrak's only to render.
//! Design: `planning/MARKDOWN_PLAN.md`; user guide: `docs/markdown.md`.

pub mod cfml;
pub mod html_in;
pub mod inline;
pub mod object;
pub mod ops;
pub mod options;
pub mod parse;
pub mod render;
pub mod table;
pub mod tree;

use cfml_common::dynamic::CfmlValue;
use cfml_common::vm::CfmlResult;

pub use options::{md_err, set_defaults, MdOptions};

/// `markdown( markdown [, options] )` — markdown in, HTML out. BoxLang's
/// `bx-markdown` shape; `options` is a superset.
pub fn fn_markdown(args: Vec<CfmlValue>) -> CfmlResult {
    let src = args.first().map(|v| v.as_string()).unwrap_or_default();
    let opts = MdOptions::overlay(&options::defaults(), args.get(1))?;
    Ok(CfmlValue::string(markdown_to_html(&src, &opts)))
}

pub fn markdown_to_html(src: &str, opts: &MdOptions) -> String {
    let owned;
    let src = if opts.dedent {
        owned = options::dedent(src);
        owned.as_str()
    } else {
        src
    };
    let html = comrak::markdown_to_html(src, &opts.comrak());
    render::apply_table_class(html, &opts.table_class)
}

/// `htmlToMarkdown( html [, options] )` — HTML in, markdown out.
pub fn fn_html_to_markdown(args: Vec<CfmlValue>) -> CfmlResult {
    let html = args.first().map(|v| v.as_string()).unwrap_or_default();
    let opts = MdOptions::overlay(&options::defaults(), args.get(1))?;
    let doc = html_in::html_to_doc(&html, &opts);
    Ok(CfmlValue::string(render::render_markdown(&doc, &opts)))
}

/// `markdownEscape( text )` — backslash-escape `text` so it reads literally
/// wherever it is dropped into markdown.
pub fn fn_markdown_escape(args: Vec<CfmlValue>) -> CfmlResult {
    let text = args.first().map(|v| v.as_string()).unwrap_or_default();
    Ok(CfmlValue::string(inline::escape_literal(&text)))
}

/// `MarkdownDocument( [source] [, options] [, html] )`.
pub fn fn_markdown_document(args: Vec<CfmlValue>) -> CfmlResult {
    object::construct(args)
}

/// `isMarkdownDocument( value )`.
pub fn fn_is_markdown_document(args: Vec<CfmlValue>) -> CfmlResult {
    Ok(CfmlValue::Bool(args.first().map(object::is_markdown_document).unwrap_or(false)))
}
