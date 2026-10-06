//! `.table( data, … )`: a GFM table from a query, an array of structs, or an
//! array of arrays — the same polymorphic `data` `spreadsheetAddRows` and the
//! Typst `Document().table()` take.
//!
//! **Data is text, not markdown.** A product called `C*` or `__init__` must
//! not render in italics or bold, so cell values are literal unless opted in,
//! at three levels where the most specific wins:
//!
//! * cell — a run in the data: `{ markdown = v }` or `{ text = v }`;
//! * column — `columnFormats = { notes = { format = "markdown" } }`;
//! * table — `cellFormat = "markdown"`;
//! * default — plain text.
//!
//! `columnFormats` has the Typst builder's shape, so it also carries `align`
//! and the `numberFormat` / `dateFormat` masks.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::CfmlError;

use super::cfml::{breaks_to_spaces, list_values, parse_align_list, Loader};
use super::options::md_err;
use super::tree::{normalise_inlines, Align, Kind, Node};

#[derive(Clone, Copy, PartialEq)]
enum Format {
    Text,
    Markdown,
}

fn parse_format(v: &CfmlValue, what: &str) -> Result<Format, CfmlError> {
    match v.as_string().trim().to_ascii_lowercase().as_str() {
        "" | "text" | "plain" | "literal" => Ok(Format::Text),
        "markdown" | "md" => Ok(Format::Markdown),
        other => Err(md_err(format!("[{}] must be \"text\" or \"markdown\", not [{}].", what, other))),
    }
}

#[derive(Clone)]
struct Column {
    /// Key into a struct / query row; `None` for array-of-arrays data.
    name: Option<String>,
    format: Option<Format>,
    align: Option<Align>,
    number_format: Option<String>,
    date_format: Option<String>,
}

pub struct TableArgs<'a> {
    pub data: &'a CfmlValue,
    pub column_list: Option<&'a CfmlValue>,
    pub headers: Option<&'a CfmlValue>,
    pub align: Option<&'a CfmlValue>,
    pub cell_format: Option<&'a CfmlValue>,
    pub column_formats: Option<&'a CfmlValue>,
}

fn present(v: Option<&CfmlValue>) -> Option<&CfmlValue> {
    v.filter(|x| !matches!(x, CfmlValue::Null) && !(matches!(x, CfmlValue::String(s) if s.trim().is_empty())))
}

pub fn build_table(loader: &mut Loader, a: TableArgs) -> Result<Node, CfmlError> {
    let table_format = match present(a.cell_format) {
        Some(v) => parse_format(v, "cellFormat")?,
        None => Format::Text,
    };
    let explicit_columns: Option<Vec<String>> =
        present(a.column_list).map(|v| list_values(v).iter().map(|c| c.as_string().trim().to_string()).filter(|c| !c.is_empty()).collect());

    // Rows of raw values, plus the header labels the data itself supplies.
    let (mut columns, data_header, rows): (Vec<Column>, Option<Vec<CfmlValue>>, Vec<Vec<CfmlValue>>) = match a.data {
        CfmlValue::Query(q) => {
            let names = explicit_columns.clone().unwrap_or_else(|| q.columns());
            let rows = q
                .rows()
                .iter()
                .map(|r| names.iter().map(|c| lookup_ci(r, c)).collect())
                .collect();
            (named_columns(&names), Some(names.iter().map(|n| CfmlValue::string(n.clone())).collect()), rows)
        }
        CfmlValue::Array(arr) => {
            let elems = arr.snapshot();
            match elems.first() {
                Some(CfmlValue::Struct(_)) => {
                    let names = match &explicit_columns {
                        Some(c) => c.clone(),
                        None => elems[0].as_struct().unwrap_or_default().keys().map(|k| k.as_str().to_string()).collect(),
                    };
                    let mut rows = Vec::new();
                    for (i, e) in elems.iter().enumerate() {
                        let m = e.as_struct().ok_or_else(|| {
                            md_err(format!("Table data row {} is a {}; every row of an array of structs must be a struct.", i + 1, e.type_name()))
                        })?;
                        rows.push(names.iter().map(|c| lookup_ci(&m, c)).collect());
                    }
                    (named_columns(&names), Some(names.iter().map(|n| CfmlValue::string(n.clone())).collect()), rows)
                }
                Some(CfmlValue::Array(_)) => {
                    let mut rows: Vec<Vec<CfmlValue>> = Vec::new();
                    for (i, e) in elems.iter().enumerate() {
                        rows.push(e.as_array().ok_or_else(|| {
                            md_err(format!("Table data row {} is a {}; every row of an array of arrays must be an array.", i + 1, e.type_name()))
                        })?);
                    }
                    // Without `headers`, the first row is the header.
                    let header = if present(a.headers).is_none() && !rows.is_empty() { Some(rows.remove(0)) } else { None };
                    let width = header.as_ref().map(|h| h.len()).into_iter().chain(rows.iter().map(|r| r.len())).max().unwrap_or(0);
                    for r in rows.iter_mut() {
                        r.resize(width, CfmlValue::Null);
                    }
                    let header = header.map(|mut h| {
                        h.resize(width, CfmlValue::Null);
                        h
                    });
                    let cols = (0..width)
                        .map(|_| Column { name: None, format: None, align: None, number_format: None, date_format: None })
                        .collect();
                    (cols, header, rows)
                }
                None => {
                    let names = explicit_columns.clone().unwrap_or_default();
                    (named_columns(&names), Some(names.iter().map(|n| CfmlValue::string(n.clone())).collect()), Vec::new())
                }
                Some(other) => {
                    return Err(md_err(format!(
                        "Table data must be a query, an array of structs or an array of arrays; this array holds a {}.",
                        other.type_name()
                    )))
                }
            }
        }
        other => {
            return Err(md_err(format!(
                "Table data must be a query, an array of structs or an array of arrays, not a {}.",
                other.type_name()
            )))
        }
    };

    // Header row: `headers` is written by the developer (inline markdown);
    // labels that came from the data are literal.
    let header_cells: Vec<(CfmlValue, bool)> = match present(a.headers) {
        Some(h) => {
            let hs = list_values(h);
            if columns.is_empty() {
                columns = (0..hs.len())
                    .map(|_| Column { name: None, format: None, align: None, number_format: None, date_format: None })
                    .collect();
            }
            if hs.len() != columns.len() {
                return Err(md_err(format!("[headers] has {} entries; the table has {} columns.", hs.len(), columns.len())));
            }
            hs.into_iter().map(|v| (v, true)).collect()
        }
        None => match data_header {
            Some(h) => h.into_iter().map(|v| (v, false)).collect(),
            None => vec![(CfmlValue::string(""), false); columns.len()],
        },
    };
    if columns.is_empty() {
        return Err(md_err("A table needs at least one column: give data with columns, columnList, or headers."));
    }

    if let Some(al) = present(a.align) {
        let aligns = parse_align_list(al)?;
        if aligns.len() > columns.len() {
            return Err(md_err(format!("[align] has {} entries; the table has {} columns.", aligns.len(), columns.len())));
        }
        for (c, al) in columns.iter_mut().zip(aligns) {
            c.align = Some(al);
        }
    }

    if let Some(cf) = present(a.column_formats) {
        let map = cf.as_struct().ok_or_else(|| md_err("[columnFormats] must be a struct keyed by column name or number."))?;
        for (k, spec) in map.iter() {
            let key = k.as_str();
            let idx = match key.trim().parse::<usize>() {
                Ok(n) if n >= 1 && n <= columns.len() => n - 1,
                Ok(n) => return Err(md_err(format!("[columnFormats] column {} is out of range: the table has {} columns.", n, columns.len()))),
                Err(_) => columns
                    .iter()
                    .position(|c| c.name.as_deref().map(|n| n.eq_ignore_ascii_case(key)).unwrap_or(false))
                    .ok_or_else(|| md_err(format!("[columnFormats] names a column [{}] that the table does not have.", key)))?,
            };
            apply_column_format(&mut columns[idx], spec, key)?;
        }
    }

    let align: Vec<Align> = columns.iter().map(|c| c.align.unwrap_or(Align::None)).collect();
    let mut table_rows = Vec::with_capacity(rows.len() + 1);
    let mut header = Vec::with_capacity(columns.len());
    for (v, authored) in &header_cells {
        let inl = if *authored {
            loader.single_line_inlines(v)?
        } else {
            literal_inlines(v)
        };
        header.push(Node::with_children(Kind::TableCell, inl));
    }
    table_rows.push(Node::with_children(Kind::TableRow, header));
    for row in rows {
        let mut cells = Vec::with_capacity(columns.len());
        for (c, v) in columns.iter().zip(row.iter()) {
            cells.push(Node::with_children(Kind::TableCell, data_cell(loader, c, v, table_format)?));
        }
        table_rows.push(Node::with_children(Kind::TableRow, cells));
    }
    Ok(Node::with_children(Kind::Table { align }, table_rows))
}

fn named_columns(names: &[String]) -> Vec<Column> {
    names
        .iter()
        .map(|n| Column { name: Some(n.clone()), format: None, align: None, number_format: None, date_format: None })
        .collect()
}

fn lookup_ci(m: &ValueMap, key: &str) -> CfmlValue {
    m.get(key).cloned().unwrap_or(CfmlValue::Null)
}

fn apply_column_format(c: &mut Column, spec: &CfmlValue, key: &str) -> Result<(), CfmlError> {
    let m = spec.as_struct().ok_or_else(|| {
        md_err(format!("[columnFormats] entry [{}] must be a struct such as {{ format = \"markdown\", align = \"right\" }}.", key))
    })?;
    for (k, v) in m.iter() {
        if matches!(v, CfmlValue::Null) {
            continue;
        }
        match k.as_str().to_ascii_lowercase().as_str() {
            "format" | "cellformat" => c.format = Some(parse_format(v, "format")?),
            "align" | "alignment" | "horizontalalignment" => {
                let s = v.as_string();
                c.align = Some(Align::parse(&s).ok_or_else(|| md_err(format!("Column alignment [{}] is not one of left, center, right or none.", s)))?);
            }
            "numberformat" | "dataformat" | "mask" => c.number_format = Some(v.as_string()),
            "dateformat" => c.date_format = Some(v.as_string()),
            // Typst-only presentation, ignored so one columnFormats struct feeds both builders.
            "bold" | "italic" | "color" | "fontcolor" | "bgcolor" | "backgroundcolor" | "fill" | "size" | "fontsize"
            | "font" | "fontname" | "width" => {}
            other => {
                return Err(md_err(format!(
                    "Unknown key [{}] in columnFormats entry [{}]. Valid keys: format, align, numberFormat, dateFormat.",
                    other, key
                )))
            }
        }
    }
    Ok(())
}

fn literal_inlines(v: &CfmlValue) -> Vec<Node> {
    let s = v.as_string().replace("\r\n", " ").replace(['\n', '\r'], " ");
    if s.is_empty() {
        Vec::new()
    } else {
        vec![Node::text(s)]
    }
}

fn data_cell(loader: &mut Loader, c: &Column, v: &CfmlValue, table_format: Format) -> Result<Vec<Node>, CfmlError> {
    match v {
        CfmlValue::Null => return Ok(Vec::new()),
        // A run decides for itself: `{ markdown = v }` or `{ text = v }`.
        CfmlValue::Struct(_) | CfmlValue::Array(_) => {
            let mut inl = loader.inlines(v)?;
            breaks_to_spaces(&mut inl);
            normalise_inlines(&mut inl);
            return Ok(inl);
        }
        _ => {}
    }
    let mut value = v.clone();
    if let Some(mask) = &c.number_format {
        let numeric = match v {
            CfmlValue::Int(_) | CfmlValue::Double(_) => true,
            CfmlValue::String(s) => cfml_common::numeric::is_numeric_string(s.trim()),
            _ => false,
        };
        if numeric {
            if let Ok(f) = crate::builtins::fn_number_format(vec![v.clone(), CfmlValue::string(mask.clone())]) {
                value = f;
            }
        }
    }
    if let Some(mask) = &c.date_format {
        if let Ok(f) = crate::dates::fn_date_format(vec![v.clone(), CfmlValue::string(mask.clone())]) {
            value = f;
        }
    }
    match c.format.unwrap_or(table_format) {
        Format::Text => Ok(literal_inlines(&value)),
        Format::Markdown => {
            let s = value.as_string().replace("\r\n", " ").replace(['\n', '\r'], " ");
            loader.single_line_inlines(&CfmlValue::string(s))
        }
    }
}
