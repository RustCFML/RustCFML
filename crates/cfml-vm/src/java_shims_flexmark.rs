//! flexmark (`com.vladsch.flexmark.*`) shimmed onto RustCFML's built-in
//! markdown engine.
//!
//! ColdBox's `cbmarkdown` module — vendored into Preside extensions, so it
//! arrives in most Preside apps — builds a flexmark pipeline through
//! cbjavaloader at module load:
//!
//! ```cfml
//! variables.StaticParser = javaloader.create( "com.vladsch.flexmark.parser.Parser" );
//! variables.HtmlRenderer = javaloader.create( "com.vladsch.flexmark.html.HtmlRenderer" );
//! var opts = javaloader.create( "com.vladsch.flexmark.util.data.MutableDataSet" ).init()
//!              .set( staticTableExtension.CLASS_NAME, "table" )
//!              .set( StaticParser.EXTENSIONS, [ TablesExtension.create(), ... ] );
//! variables.parser   = StaticParser.builder( opts ).build();
//! variables.renderer = HtmlRenderer.builder( opts ).build();
//! ```
//!
//! There is no JVM, so every one of those calls used to die on the deferred-Java
//! error and took the whole application's boot with it. RustCFML has its own
//! markdown engine (comrak) that does everything cbmarkdown asks flexmark for,
//! so the API surface is modelled here and the actual work routed to it — the
//! app needs no source change, which is the whole point of the Java shims.
//!
//! The shapes:
//!
//! | kind        | is                                      | answers                     |
//! |-------------|-----------------------------------------|-----------------------------|
//! | `class`     | a flexmark class handle (`Parser`, …)   | `create`, `builder`, `init`, and its static option keys as members |
//! | `dataset`   | `MutableDataSet`                        | `init`, `set` (chainable), `get` |
//! | `builder`   | `X.builder( opts )`                     | `build`, `set`              |
//! | `parser`    | a built `Parser`                        | `parse` -> document         |
//! | `renderer`  | a built `HtmlRenderer`                  | `render( doc )` -> HTML     |
//! | `converter` | a built `FlexmarkHtmlConverter`         | `convert( html )` -> markdown |
//! | `document`  | `parser.parse( txt )`                   | carries the source text     |
//! | `extension` | `TablesExtension.create()` and friends  | nothing; it is a marker     |
//!
//! Option keys are tokens (`"tables.CLASS_NAME"`), not live Java objects, so a
//! `.set( key, value )` chain lands in one flat map that
//! [`options_from_dataset`] folds into our own `MdOptions`.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};

/// `__java_class` for every flexmark shim; the `__fm_kind` key tells them apart.
pub const FLEXMARK_CLASS: &str = "com.vladsch.flexmark.__shim";

/// Is this a flexmark class the shim stands in for?
pub fn is_flexmark_class(class_name: &str) -> bool {
    class_name.starts_with("com.vladsch.flexmark.")
}

fn shim(kind: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert(
        "__java_class".to_string(),
        CfmlValue::string(FLEXMARK_CLASS.to_string()),
    );
    m.insert("__fm_kind".to_string(), CfmlValue::string(kind.to_string()));
    m
}

/// The short name a flexmark class is known by here: the last segment, minus
/// the `Extension` suffix, lowercased — `…ext.tables.TablesExtension` ->
/// `tables`, `…ext.gfm.tasklist.TaskListExtension` -> `tasklist`.
fn short_name(class_name: &str) -> String {
    let last = class_name.rsplit('.').next().unwrap_or(class_name);
    let base = last.strip_suffix("Extension").unwrap_or(last);
    base.to_ascii_lowercase()
}

/// The static option keys a flexmark class exposes as public fields. They are
/// read as members (`TablesExtension.CLASS_NAME`) and handed straight back to
/// `MutableDataSet.set`, so a token string carries all the meaning needed.
/// Listed per class rather than synthesised, so a typo in app code still reads
/// as null the way it would on Lucee.
const STATIC_KEYS: &[(&str, &[&str])] = &[
    (
        "parser",
        &[
            "EXTENSIONS",
            "WWW_AUTO_LINK_ELEMENT",
            "HTML_BLOCK_DEEP_PARSER",
            "BLANK_LINES_IN_AST",
            "HEADING_NO_ATX_SPACE",
            "LISTS_AUTO_LOOSE",
        ],
    ),
    (
        "htmlrenderer",
        &[
            "CODE_STYLE_HTML_OPEN",
            "CODE_STYLE_HTML_CLOSE",
            "FENCED_CODE_LANGUAGE_CLASS_PREFIX",
            "SOFT_BREAK",
            "HARD_BREAK",
            "ESCAPE_HTML",
            "SUPPRESS_HTML",
            "GENERATE_HEADER_ID",
            "RENDER_HEADER_ID",
            "INDENT_SIZE",
        ],
    ),
    (
        "tables",
        &[
            "COLUMN_SPANS",
            "APPEND_MISSING_COLUMNS",
            "DISCARD_EXTRA_COLUMNS",
            "CLASS_NAME",
            "HEADER_SEPARATOR_COLUMN_MATCH",
            "WITH_CAPTION",
            "MIN_HEADER_ROWS",
            "MAX_HEADER_ROWS",
        ],
    ),
    (
        "anchorlink",
        &[
            "ANCHORLINKS_SET_ID",
            "ANCHORLINKS_SET_NAME",
            "ANCHORLINKS_WRAP_TEXT",
            "ANCHORLINKS_ANCHOR_CLASS",
            "ANCHORLINKS_TEXT_PREFIX",
            "ANCHORLINKS_TEXT_SUFFIX",
            "ANCHORLINKS_NO_BLOCK_TEXT",
        ],
    ),
    ("toc", &["LEVELS", "TITLE", "TITLE_LEVEL", "DIV_CLASS"]),
    ("tasklist", &["ITEM_DONE_MARKER", "ITEM_NOT_DONE_MARKER"]),
    ("autolink", &["IGNORE_LINKS"]),
];

/// A flexmark class handle: `javaloader.create( "com.vladsch.flexmark.…" )`.
/// Its static option keys are ordinary members, which is how
/// `TablesExtension.CLASS_NAME` reads without any member-access special case.
pub fn make_flexmark_class(class_name: &str) -> CfmlValue {
    let short = short_name(class_name);
    let mut m = shim("class");
    m.insert(
        "__class_name".to_string(),
        CfmlValue::string(class_name.to_string()),
    );
    m.insert("__fm_name".to_string(), CfmlValue::string(short.clone()));
    if let Some((_, keys)) = STATIC_KEYS.iter().find(|(c, _)| *c == short) {
        for key in *keys {
            m.insert(
                key.to_string(),
                CfmlValue::string(format!("{}.{}", short, key)),
            );
        }
    }
    CfmlValue::strukt(m)
}

fn make_with_opts(kind: &str, opts: CfmlValue) -> CfmlValue {
    let mut m = shim(kind);
    m.insert("__fm_opts".to_string(), opts);
    CfmlValue::strukt(m)
}

/// Dispatch a method on any flexmark shim. `Err(shim_unhandled)` falls through
/// to the generic member dispatch, as everywhere else in the shim layer.
pub fn handle_flexmark(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let s = match object {
        CfmlValue::Struct(s) => s,
        _ => return Err(CfmlError::shim_unhandled(method)),
    };
    let get = |k: &str| s.get(k).unwrap_or(CfmlValue::Null);
    let kind = get("__fm_kind").as_string();
    let name = get("__fm_name").as_string();
    let arg0 = args.first().cloned().unwrap_or(CfmlValue::Null);

    match (kind.as_str(), method) {
        // --- the class handle ---
        // `TablesExtension.create()` — flexmark's static extension factory.
        ("class", "create") => {
            let mut m = shim("extension");
            m.insert("__fm_ext".to_string(), CfmlValue::string(name));
            Ok(CfmlValue::strukt(m))
        }
        // `Parser.builder( opts )` / `HtmlRenderer.builder( opts )` /
        // `FlexmarkHtmlConverter.builder( opts )`.
        ("class", "builder") => {
            let mut m = shim("builder");
            m.insert("__fm_for".to_string(), CfmlValue::string(name));
            m.insert("__fm_opts".to_string(), arg0);
            Ok(CfmlValue::strukt(m))
        }
        // `MutableDataSet.init()` — and any other flexmark class instantiated
        // with no arguments, which is only ever the data set in practice.
        ("class", "init") => Ok(CfmlValue::strukt(shim("dataset"))),

        // --- MutableDataSet ---
        ("dataset", "init") => Ok(object.clone()),
        // `.set( key, value )` is CHAINABLE in flexmark (it returns the data
        // set), which is the whole shape of cbmarkdown's createOptions().
        ("dataset", "set") => {
            let key = arg0.as_string();
            let val = args.get(1).cloned().unwrap_or(CfmlValue::Null);
            let mut m = s.snapshot();
            m.insert(format!("__fm_opt_{}", key.to_ascii_lowercase()), val);
            Ok(CfmlValue::strukt(m))
        }
        ("dataset", "get") => Ok(s
            .get(&format!("__fm_opt_{}", arg0.as_string().to_ascii_lowercase()))
            .unwrap_or(CfmlValue::Null)),
        // `.toImmutable()` / `.toMutable()` — same bag either way here.
        ("dataset", "toimmutable") | ("dataset", "tomutable") => Ok(object.clone()),

        // --- builders ---
        ("builder", "build") => {
            let opts = get("__fm_opts");
            let built = match get("__fm_for").as_string().as_str() {
                "parser" => "parser",
                "flexmarkhtmlconverter" => "converter",
                _ => "renderer",
            };
            Ok(make_with_opts(built, opts))
        }
        // A builder takes `.set()` too, and keeps being a builder.
        ("builder", "set") => {
            let mut m = s.snapshot();
            let opts = match m.get("__fm_opts") {
                Some(CfmlValue::Struct(ds)) => {
                    let mut d = ds.snapshot();
                    d.insert(
                        format!("__fm_opt_{}", arg0.as_string().to_ascii_lowercase()),
                        args.get(1).cloned().unwrap_or(CfmlValue::Null),
                    );
                    CfmlValue::strukt(d)
                }
                _ => get("__fm_opts"),
            };
            m.insert("__fm_opts".to_string(), opts);
            Ok(CfmlValue::strukt(m))
        }
        ("builder", "extensions") => Ok(object.clone()),

        // --- the working objects ---
        // `parser.parse( txt )` returns flexmark's Node tree. Nothing in
        // cbmarkdown inspects it — it goes straight to `renderer.render()` —
        // so the "document" carries its source and the render does the work in
        // one pass.
        ("parser", "parse") => {
            let mut m = shim("document");
            m.insert("__fm_src".to_string(), CfmlValue::string(arg0.as_string()));
            Ok(CfmlValue::strukt(m))
        }
        // `renderer.render( doc )` and `converter.convert( html )` are
        // answered by the VM (`dispatch_flexmark`), which has the markdown
        // builtins; they never reach here.

        // Reflection-ish odds and ends every shim answers.
        ("class", "getname") | ("class", "getcanonicalname") => Ok(get("__class_name")),
        (_, "tostring") => Ok(CfmlValue::string(format!("flexmark:{}", kind))),
        _ => Err(CfmlError::shim_unhandled(method)),
    }
}

/// Fold a flexmark data set into the options struct our `markdown()` /
/// `htmlToMarkdown()` builtins take.
///
/// Only the keys that mean something to our engine are honoured; the rest
/// (flexmark's AST and formatter knobs) have no effect, which is the honest
/// outcome — they describe flexmark's internals, not the output contract.
pub fn options_struct_from_dataset(dataset: &CfmlValue) -> CfmlValue {
    let mut o = ValueMap::default();
    // flexmark renders raw HTML through, where our engine defaults to comrak's
    // safe mode. A markdown pipeline the app wired up itself is trusted input,
    // so match flexmark rather than silently eating the app's own HTML.
    o.insert("unsafe".to_string(), CfmlValue::Bool(true));
    let CfmlValue::Struct(s) = dataset else {
        return CfmlValue::strukt(o);
    };
    for (k, v) in s.iter() {
        let Some(key) = k.strip_prefix("__fm_opt_") else {
            continue;
        };
        match key {
            "tables.class_name" => {
                o.insert("tableClass".to_string(), CfmlValue::string(v.as_string()));
            }
            "anchorlink.anchorlinks_set_id" => {
                o.insert("anchors".to_string(), CfmlValue::Bool(v.is_true()));
            }
            "htmlrenderer.escape_html" => {
                o.insert("escapeHtml".to_string(), CfmlValue::Bool(v.is_true()));
            }
            "htmlrenderer.suppress_html" => {
                if v.is_true() {
                    o.insert("unsafe".to_string(), CfmlValue::Bool(false));
                }
            }
            // `Parser.EXTENSIONS` is the array of `X.create()` markers. The
            // ones our engine can switch on are switched on; GFM covers
            // tables/strikethrough/tasklist/autolink together.
            "parser.extensions" => {
                for ext in v.as_array_or_query_column().unwrap_or_default() {
                    if let CfmlValue::Struct(e) = ext {
                        let name = e
                            .get("__fm_ext")
                            .map(|n| n.as_string())
                            .unwrap_or_default();
                        match name.as_str() {
                            "tables" | "strikethroughsubscript" | "strikethrough"
                            | "tasklist" | "autolink" => {
                                o.insert("gfm".to_string(), CfmlValue::Bool(true));
                            }
                            "anchorlink" => {
                                o.insert("anchors".to_string(), CfmlValue::Bool(true));
                            }
                            "yamlfrontmatter" => {
                                o.insert("frontMatter".to_string(), CfmlValue::Bool(true));
                            }
                            "footnotes" => {
                                o.insert("footnotes".to_string(), CfmlValue::Bool(true));
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
    CfmlValue::strukt(o)
}

/// The markdown source a `parser.parse()` document carries (or the value
/// itself, for app code that hands the renderer a plain string).
pub fn document_source(v: &CfmlValue) -> String {
    match v {
        CfmlValue::Struct(d) => d
            .get("__fm_src")
            .map(|s| s.as_string())
            .unwrap_or_else(|| v.as_string()),
        other => other.as_string(),
    }
}
