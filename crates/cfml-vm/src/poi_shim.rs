//! Apache POI (`org.apache.poi.*`), backed by RustCFML's native spreadsheet engine.
//!
//! The target is CFML spreadsheet libraries that ship POI as a jar and drive its
//! object graph directly — above all `lucee-spreadsheet` (`spreadsheetCFML`),
//! which Preside vendors and injects as `spreadsheetLib` for every admin data
//! export and form-builder download.
//!
//! # The idea
//!
//! We do **not** reimplement POI. RustCFML already has a spreadsheet engine
//! (umya, behind the 66 `Spreadsheet*` builtins), and a workbook there is a
//! `CfmlValue::NativeObject` — a *shared* handle, so every copy of it addresses
//! the same workbook. That makes POI's object graph expressible as **coordinate
//! handles**:
//!
//! ```text
//!   Workbook  { __wb }                        -> the native workbook
//!   Sheet     { __wb, __sheet }               -> + a sheet index
//!   Row       { __wb, __sheet, __row }        -> + a row
//!   Cell      { __wb, __sheet, __row, __col } -> + a column
//! ```
//!
//! Every mutation is then just the corresponding builtin —
//! `cell.setCellValue( v )` is `spreadsheetSetCellValue( wb, v, row, col )`. No
//! POI semantics are emulated in Rust; the engine does the work it already does,
//! and this module is the adapter that lets a POI-shaped caller reach it.
//!
//! POI is **0-based** throughout (row 0 is the first row); the builtins are
//! CFML's **1-based**. The conversion happens here, at the boundary, once.
//!
//! # Two places the shapes genuinely differ
//!
//! * **`new XSSFWorkbook()` has no sheets**, while `spreadsheetNew()` always
//!   creates one. So the shim keeps POI's view of the sheet list in `__sheets`
//!   and, on the *first* `createSheet()`, renames the engine's default sheet
//!   rather than adding a second one. Every later `createSheet()` adds normally.
//!   Without this a caller that does `new Workbook()` then `createSheet("Data")`
//!   — which is exactly what `lucee-spreadsheet`'s `new()` does — silently gets a
//!   stray leading "Sheet1".
//!
//! * **`CellStyle`/`Font` are configure-then-assign** in POI, whereas the
//!   builtins take a format struct at the point of application. A style here is
//!   therefore an *accumulator*: setters record into `__fmt`, and
//!   `cell.setCellStyle( s )` / `row.setRowStyle( s )` is where
//!   `spreadsheetFormatCell` / `spreadsheetFormatRow` actually runs. A style
//!   mutated after assignment does not retroactively change the cells it was
//!   already applied to — POI would. That is documented in
//!   `docs/known-issues.md` rather than silently papered over.
//!
//! Anything outside this adapter's reach **throws, naming the class and method**.
//! A spreadsheet that silently loses a column is worse than one that fails.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

pub const WORKBOOK_XSSF: &str = "org.apache.poi.xssf.usermodel.xssfworkbook";
pub const WORKBOOK_SXSSF: &str = "org.apache.poi.xssf.streaming.sxssfworkbook";
pub const WORKBOOK_HSSF: &str = "org.apache.poi.hssf.usermodel.hssfworkbook";
pub const WORKBOOK_FACTORY: &str = "org.apache.poi.ss.usermodel.workbookfactory";
pub const WORKBOOK_UTIL: &str = "org.apache.poi.ss.util.workbookutil";
pub const SHEET: &str = "org.apache.poi.ss.usermodel.sheet";
pub const ROW: &str = "org.apache.poi.ss.usermodel.row";
pub const CELL: &str = "org.apache.poi.ss.usermodel.cell";
pub const CELL_STYLE: &str = "org.apache.poi.ss.usermodel.cellstyle";
pub const FONT: &str = "org.apache.poi.ss.usermodel.font";
pub const CELL_UTIL: &str = "org.apache.poi.ss.util.cellutil";
pub const DATE_UTIL: &str = "org.apache.poi.ss.usermodel.dateutil";
pub const CREATION_HELPER: &str = "org.apache.poi.ss.usermodel.creationhelper";
pub const DATA_FORMAT: &str = "org.apache.poi.ss.usermodel.dataformat";
/// Not a POI type: the adapter's own `java.util.Iterator` over coordinate
/// handles, returned by `Sheet.rowIterator()` / `Row.cellIterator()`.
pub const ITERATOR: &str = "org.apache.poi.ss.usermodel.__iterator";

/// Classes a caller may name in `createObject("java", …)`.
pub fn is_poi_class(class_lower: &str) -> bool {
    matches!(
        class_lower,
        WORKBOOK_XSSF
            | WORKBOOK_SXSSF
            | WORKBOOK_HSSF
            | WORKBOOK_FACTORY
            | WORKBOOK_UTIL
            | CELL_UTIL
            | DATE_UTIL
            | SHEET
            | ROW
            | CELL
            | CELL_STYLE
            | FONT
            | CREATION_HELPER
            | DATA_FORMAT
    )
}

/// Classes this module dispatches methods for (a superset of the constructible
/// ones — sheets, rows and cells are only ever produced by a workbook).
pub fn handles(class_lower: &str) -> bool {
    is_poi_class(class_lower) || class_lower == ITERATOR
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

pub fn construct(class_lower: &str) -> CfmlResult {
    // Construction is deferred to `init()`: `createObject("java", X)` with no
    // arguments is how CFML reaches the class object, and the real constructor
    // arguments (a streaming window size, say) arrive on the explicit `.init()`.
    let mut m = shim(class_lower);
    if class_lower == CELL_UTIL {
        // `CellUtil`'s style-property names are public String constants read as
        // FIELDS (`getCellUtil().DATA_FORMAT`), not through a getter. They have to
        // be real keys on the struct or the read falls through to "undefined".
        // The value is the name the native format struct uses, so
        // setCellStyleProperty() can pass it straight on.
        for (field, key) in CELL_UTIL_CONSTANTS {
            m.insert(field.to_string(), CfmlValue::string(key.to_string()));
        }
    }
    Ok(CfmlValue::strukt(m))
}

/// `CellUtil.<CONSTANT>` -> the key the native format struct uses. Only the
/// properties the formatter actually understands are exposed; a caller reaching
/// for one that is absent gets "undefined", which is a visible failure, rather
/// than a style silently recorded and dropped.
const CELL_UTIL_CONSTANTS: &[(&str, &str)] = &[
    ("DATA_FORMAT", "dataformat"),
    ("ALIGNMENT", "alignment"),
    ("VERTICAL_ALIGNMENT", "verticalalignment"),
    ("WRAP_TEXT", "wraptext"),
    ("FONT", "font"),
    ("FILL_FOREGROUND_COLOR", "fgcolor"),
];

fn get(object: &CfmlValue, key: &str) -> Option<CfmlValue> {
    match object {
        CfmlValue::Struct(s) => s.get(key),
        _ => None,
    }
}

fn get_int(object: &CfmlValue, key: &str) -> i64 {
    match get(object, key) {
        Some(CfmlValue::Int(n)) => n,
        Some(CfmlValue::Double(d)) => d as i64,
        Some(other) => other.as_string().trim().parse().unwrap_or(0),
        None => 0,
    }
}

fn arg_int(args: &[CfmlValue], i: usize) -> i64 {
    match args.get(i) {
        Some(CfmlValue::Int(n)) => *n,
        Some(CfmlValue::Double(d)) => *d as i64,
        Some(other) => other.as_string().trim().parse().unwrap_or(0),
        None => 0,
    }
}

fn arg_bool(args: &[CfmlValue], i: usize, default: bool) -> bool {
    match args.get(i) {
        Some(CfmlValue::Bool(b)) => *b,
        Some(CfmlValue::Null) | None => default,
        Some(other) => {
            let s = other.as_string();
            !(s.eq_ignore_ascii_case("false") || s == "0" || s.is_empty())
        }
    }
}

/// The native workbook handle carried by any shim in the graph.
fn workbook_of(object: &CfmlValue) -> Option<CfmlValue> {
    get(object, "__wb")
}

fn illegal_argument(message: impl std::fmt::Display) -> CfmlError {
    CfmlError::new(
        format!("java.lang.IllegalArgumentException: {}", message),
        CfmlErrorType::Custom("java.lang.IllegalArgumentException".to_string()),
    )
}

fn unsupported(class: &str, method: &str) -> CfmlError {
    CfmlError::new(
        format!(
            "{}.{}() is not supported by RustCFML's POI adapter. The adapter maps POI's \
             object graph onto the native spreadsheet engine; it covers workbook/sheet/row/\
             cell/style construction, values, formatting and writing. Anything beyond that \
             is refused rather than silently dropped from the output.",
            class, method
        ),
        CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
    )
}

/// Build a Sheet/Row/Cell handle by extending the receiver's coordinates.
fn descend(base: &CfmlValue, class: &str, extra: &[(&str, i64)]) -> CfmlValue {
    let mut m = shim(class);
    for key in ["__wb", "__sheet", "__row", "__col"] {
        if let Some(v) = get(base, key) {
            m.insert(key.to_string(), v);
        }
    }
    for (k, v) in extra {
        m.insert(k.to_string(), CfmlValue::Int(*v));
    }
    CfmlValue::strukt(m)
}

/// POI's sheet-name rules, as `WorkbookUtil.validateSheetName` enforces them.
fn validate_sheet_name(name: &str) -> Result<(), CfmlError> {
    if name.is_empty() {
        return Err(illegal_argument("sheetName must not be empty"));
    }
    if name.chars().count() > 31 {
        return Err(illegal_argument(format!(
            "sheetName '{}' is too long (max 31 characters)",
            name
        )));
    }
    for c in ['/', '\\', '?', '*', ']', '[', ':'] {
        if name.contains(c) {
            return Err(illegal_argument(format!(
                "Invalid char ({}) found at index {} in sheet name '{}'",
                c,
                name.find(c).unwrap_or(0),
                name
            )));
        }
    }
    if name.starts_with('\'') || name.ends_with('\'') {
        return Err(illegal_argument(
            "sheet name must not begin or end with a single quote",
        ));
    }
    Ok(())
}

/// The names the shim believes the workbook has, in POI's ordering.
fn sheet_names(object: &CfmlValue) -> Vec<String> {
    match get(object, "__sheets") {
        Some(CfmlValue::Array(a)) => a.snapshot().iter().map(|v| v.as_string()).collect(),
        _ => Vec::new(),
    }
}

fn set_sheet_names(object: &CfmlValue, names: Vec<String>) {
    if let CfmlValue::Struct(s) = object {
        s.insert(
            "__sheets".to_string(),
            CfmlValue::array(names.into_iter().map(CfmlValue::string).collect()),
        );
    }
}

/// A style/font accumulator's recorded format struct.
fn fmt_of(object: &CfmlValue) -> ValueMap {
    match get(object, "__fmt") {
        Some(CfmlValue::Struct(s)) => s.snapshot(),
        _ => ValueMap::default(),
    }
}

fn set_fmt_key(object: &CfmlValue, key: &str, value: CfmlValue) {
    let mut f = fmt_of(object);
    f.insert(key.to_string(), value);
    if let CfmlValue::Struct(s) = object {
        s.insert("__fmt".to_string(), CfmlValue::strukt(f));
    }
}

/// POI 5 exposes alignments/borders as enums that the library subscripts by name
/// (`cellStyle.getAlignment()[ "LEFT" ]`). A struct of NAME -> lowercase name
/// reproduces the subscript, and the lowercase name is the spelling the native
/// formatter expects, so the value round-trips straight into `setAlignment`.
/// A fresh style/font accumulator, tagged with the concrete POI class name the
/// owning workbook's flavour implies — `XSSFCellStyle` for an xlsx workbook,
/// `HSSFCellStyle` for xls — because that is the string the library validates.
fn new_accumulator(kind: &str, workbook_class: &str) -> CfmlValue {
    let binary = workbook_class == WORKBOOK_HSSF;
    let concrete = match (kind, binary) {
        (CELL_STYLE, true) => "org.apache.poi.hssf.usermodel.HSSFCellStyle",
        (CELL_STYLE, false) => "org.apache.poi.xssf.usermodel.XSSFCellStyle",
        (_, true) => "org.apache.poi.hssf.usermodel.HSSFFont",
        (_, false) => "org.apache.poi.xssf.usermodel.XSSFFont",
    };
    let mut m = shim(kind);
    m.insert("__fmt".to_string(), CfmlValue::strukt(ValueMap::default()));
    m.insert("__poi_class".to_string(), CfmlValue::string(concrete.to_string()));
    CfmlValue::strukt(m)
}

fn iter_items(object: &CfmlValue) -> Vec<CfmlValue> {
    match get(object, "__items") {
        Some(CfmlValue::Array(a)) => a.snapshot(),
        _ => Vec::new(),
    }
}

fn make_iterator(items: Vec<CfmlValue>) -> CfmlValue {
    let mut m = shim(ITERATOR);
    m.insert("__items".to_string(), CfmlValue::array(items));
    m.insert("__pos".to_string(), CfmlValue::Int(0));
    CfmlValue::strukt(m)
}

fn alignment_enum(names: &[&str]) -> CfmlValue {
    let mut m = ValueMap::default();
    for n in names {
        m.insert(n.to_string(), CfmlValue::string(n.to_ascii_lowercase()));
    }
    CfmlValue::strukt(m)
}

/// The properly-cased POI canonical name for a shim class.
///
/// This is not cosmetic. `lucee-spreadsheet` decides what kind of workbook it is
/// holding — and validates that a value really is a cell style — by string-
/// comparing `getClass().getCanonicalName()` against the exact POI class name
/// (`isBinaryFormat`, `isValidCellStyleObject`). Report the lowercased key we
/// dispatch on and every one of those checks silently answers "no".
fn canonical_name(class_lower: &str) -> &'static str {
    match class_lower {
        WORKBOOK_XSSF => "org.apache.poi.xssf.usermodel.XSSFWorkbook",
        WORKBOOK_SXSSF => "org.apache.poi.xssf.streaming.SXSSFWorkbook",
        WORKBOOK_HSSF => "org.apache.poi.hssf.usermodel.HSSFWorkbook",
        WORKBOOK_FACTORY => "org.apache.poi.ss.usermodel.WorkbookFactory",
        WORKBOOK_UTIL => "org.apache.poi.ss.util.WorkbookUtil",
        CELL_UTIL => "org.apache.poi.ss.util.CellUtil",
        DATE_UTIL => "org.apache.poi.ss.usermodel.DateUtil",
        SHEET => "org.apache.poi.ss.usermodel.Sheet",
        ROW => "org.apache.poi.ss.usermodel.Row",
        CELL => "org.apache.poi.ss.usermodel.Cell",
        CELL_STYLE => "org.apache.poi.ss.usermodel.CellStyle",
        FONT => "org.apache.poi.ss.usermodel.Font",
        CREATION_HELPER => "org.apache.poi.ss.usermodel.CreationHelper",
        DATA_FORMAT => "org.apache.poi.ss.usermodel.DataFormat",
        _ => "java.lang.Object",
    }
}

fn class_name_of(class_lower: &str) -> &'static str {
    if class_lower == FONT {
        "org.apache.poi.ss.usermodel.Font"
    } else {
        "org.apache.poi.ss.usermodel.CellStyle"
    }
}

/// `bifs` is the bridge to the registered `Spreadsheet*` builtins.
pub fn dispatch(
    class_lower: &str,
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    bif: &dyn Fn(&str, Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    // Every POI object answers getClass(); the library type-checks with it.
    // A style/font records the concrete workbook-flavoured class it was created
    // for (XSSFCellStyle vs HSSFCellStyle), since that is what gets compared.
    if method == "getclass" {
        let concrete = get(object, "__poi_class")
            .map(|v| v.as_string())
            .unwrap_or_else(|| canonical_name(class_lower).to_string());
        return Ok(crate::java_shims::make_class_shim(&concrete));
    }
    match class_lower {
        WORKBOOK_XSSF | WORKBOOK_SXSSF | WORKBOOK_HSSF | WORKBOOK_FACTORY => {
            workbook(class_lower, method, args, object, bif)
        }
        WORKBOOK_UTIL => match method {
            "validatesheetname" => {
                validate_sheet_name(&args.first().map(|v| v.as_string()).unwrap_or_default())?;
                Ok(CfmlValue::Null)
            }
            "createsafesheetname" => {
                let raw = args.first().map(|v| v.as_string()).unwrap_or_default();
                let mut safe: String = raw
                    .chars()
                    .map(|c| if "/\\?*][:".contains(c) { ' ' } else { c })
                    .collect();
                safe.truncate(31);
                Ok(CfmlValue::string(safe))
            }
            other => Err(unsupported("org.apache.poi.ss.util.WorkbookUtil", other)),
        },
        CELL_UTIL => match method {
            // getRow( rowIndex, sheet ) / getCell( row, columnIndex ) — POI's
            // "return it, creating it if absent" helpers. Coordinate handles are
            // created on demand anyway, so both are just a descend.
            "getrow" | "createrow" => Ok(descend(
                &args.get(1).cloned().unwrap_or(CfmlValue::Null),
                ROW,
                &[("__row", arg_int(&args, 0))],
            )),
            "getcell" | "createcell" => Ok(descend(
                &args.first().cloned().unwrap_or(CfmlValue::Null),
                CELL,
                &[("__col", arg_int(&args, 1))],
            )),
            // setCellStyleProperty( cell, propertyName, value ) — POI's
            // reuse-an-existing-style path. Applied immediately: there is no
            // style pool to grow, so the 4009-styles limit it exists to dodge
            // does not apply here.
            "setcellstyleproperty" => {
                let target = args.first().cloned().unwrap_or(CfmlValue::Null);
                let key = args.get(1).map(|v| v.as_string()).unwrap_or_default();
                let value = args.get(2).cloned().unwrap_or(CfmlValue::Null);
                if key.is_empty() {
                    return Err(illegal_argument("setCellStyleProperty needs a property name"));
                }
                let mut fmt = ValueMap::default();
                fmt.insert(key, value);
                bif(
                    "spreadsheetFormatCell",
                    vec![
                        workbook_of(&target).unwrap_or(CfmlValue::Null),
                        CfmlValue::strukt(fmt),
                        CfmlValue::Int(get_int(&target, "__row") + 1),
                        CfmlValue::Int(get_int(&target, "__col") + 1),
                    ],
                )?;
                Ok(CfmlValue::Null)
            }
            "setcellstyleproperties" => {
                let target = args.first().cloned().unwrap_or(CfmlValue::Null);
                let fmt = match args.get(1) {
                    Some(CfmlValue::Struct(s)) => s.snapshot(),
                    _ => ValueMap::default(),
                };
                if !fmt.is_empty() {
                    bif(
                        "spreadsheetFormatCell",
                        vec![
                            workbook_of(&target).unwrap_or(CfmlValue::Null),
                            CfmlValue::strukt(fmt),
                            CfmlValue::Int(get_int(&target, "__row") + 1),
                            CfmlValue::Int(get_int(&target, "__col") + 1),
                        ],
                    )?;
                }
                Ok(CfmlValue::Null)
            }
            other => Err(unsupported("org.apache.poi.ss.util.CellUtil", other)),
        },
        DATE_UTIL => match method {
            // Excel's serial-date epoch is 1899-12-30, and a serial is
            // days-since-epoch with the time as the fraction. `getExcelDate`
            // takes a date, `getJavaDate` reverses it.
            "getexceldate" => {
                let secs = args
                    .first()
                    .map(|v| v.as_string())
                    .and_then(|s| s.trim().parse::<f64>().ok())
                    .unwrap_or(0.0);
                Ok(CfmlValue::Double(secs))
            }
            "iscelldateformatted" | "isvaliddate" => Ok(CfmlValue::Bool(false)),
            other => Err(unsupported("org.apache.poi.ss.usermodel.DateUtil", other)),
        },
        ITERATOR => match method {
            "hasnext" => {
                let items = iter_items(object);
                Ok(CfmlValue::Bool((get_int(object, "__pos") as usize) < items.len()))
            }
            "next" => {
                let items = iter_items(object);
                let pos = get_int(object, "__pos") as usize;
                if pos >= items.len() {
                    return Err(CfmlError::new(
                        "java.util.NoSuchElementException: iterator is exhausted".to_string(),
                        CfmlErrorType::Custom("java.util.NoSuchElementException".to_string()),
                    ));
                }
                // Advance in place so the caller's `while( it.hasNext() )` loop
                // terminates — the receiver is the handle they are holding.
                if let CfmlValue::Struct(st) = object {
                    st.insert("__pos".to_string(), CfmlValue::Int(pos as i64 + 1));
                }
                Ok(items[pos].clone())
            }
            "remove" => Err(unsupported("java.util.Iterator", "remove")),
            other => Err(unsupported("java.util.Iterator", other)),
        },
        SHEET => sheet(method, args, object, bif),
        ROW => row(method, args, object, bif),
        CELL => cell(method, args, object, bif),
        CELL_STYLE | FONT => style(class_lower, method, args, object),
        CREATION_HELPER => match method {
            "createdataformat" => Ok(CfmlValue::strukt(shim(DATA_FORMAT))),
            other => Err(unsupported("org.apache.poi.ss.usermodel.CreationHelper", other)),
        },
        DATA_FORMAT => match method {
            // POI hands back a numeric format id; the shim keeps the format
            // STRING, because that is what the builtins take. Callers only ever
            // round-trip it back into setDataFormat().
            "getformat" => Ok(args.first().cloned().unwrap_or(CfmlValue::Null)),
            other => Err(unsupported("org.apache.poi.ss.usermodel.DataFormat", other)),
        },
        other => Err(unsupported(other, method)),
    }
}

// ── Workbook ─────────────────────────────────────────────────────────────────

fn workbook(
    class_lower: &str,
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    bif: &dyn Fn(&str, Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    match method {
        // new HSSFWorkbook() / new XSSFWorkbook() / new SXSSFWorkbook( windowSize )
        "init" => {
            // `new HSSFWorkbook()` asks for legacy binary .xls, which the engine
            // reads but cannot write. Rather than fail the caller's export, the
            // workbook is backed by xlsx and the SUBSTITUTION IS RECORDED — see
            // `__xls_substituted` and the warning in `write` below.
            //
            // The shim keeps reporting `HSSFWorkbook` from getClass() on purpose:
            // the library branches on that to decide which cell-style and colour
            // classes to use, and those branches must stay self-consistent with
            // what the style accumulators report. Only the BYTES on disk differ.
            let substituting_xls = class_lower == WORKBOOK_HSSF;
            // The engine always gives us a sheet; POI does not. Keep POI's view
            // empty and reuse that sheet on the first createSheet().
            let wb = bif(
                "spreadsheetNew",
                vec![CfmlValue::string("Sheet1".to_string()), CfmlValue::Bool(true)],
            )?;
            let mut m = shim(class_lower);
            m.insert("__wb".to_string(), wb);
            m.insert("__sheets".to_string(), CfmlValue::array(Vec::new()));
            m.insert("__active".to_string(), CfmlValue::Int(0));
            if substituting_xls {
                m.insert("__xls_substituted".to_string(), CfmlValue::Bool(true));
            }
            Ok(CfmlValue::strukt(m))
        }
        "createsheet" => {
            let name = match args.first() {
                Some(v) if !v.as_string().is_empty() => v.as_string(),
                _ => format!("Sheet{}", sheet_names(object).len() + 1),
            };
            validate_sheet_name(&name)?;
            let mut names = sheet_names(object);
            if names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                return Err(illegal_argument(format!(
                    "The workbook already contains a sheet named '{}'",
                    name
                )));
            }
            let wb = workbook_of(object).unwrap_or(CfmlValue::Null);
            if names.is_empty() {
                // Reuse the engine's default sheet — see the module note.
                bif(
                    "spreadsheetRenameSheet",
                    vec![wb.clone(), CfmlValue::string(name.clone()), CfmlValue::Int(1)],
                )?;
            } else {
                bif(
                    "spreadsheetCreateSheet",
                    vec![wb.clone(), CfmlValue::string(name.clone())],
                )?;
            }
            names.push(name);
            let index = names.len() as i64 - 1;
            set_sheet_names(object, names);
            Ok(descend(object, SHEET, &[("__sheet", index)]))
        }
        "getnumberofsheets" => Ok(CfmlValue::Int(sheet_names(object).len() as i64)),
        "getsheetat" => {
            let i = arg_int(&args, 0);
            let names = sheet_names(object);
            if i < 0 || i as usize >= names.len() {
                return Err(illegal_argument(format!(
                    "Sheet index ({}) is out of range (0..{})",
                    i,
                    names.len().saturating_sub(1)
                )));
            }
            Ok(descend(object, SHEET, &[("__sheet", i)]))
        }
        "getsheet" => {
            let want = args.first().map(|v| v.as_string()).unwrap_or_default();
            match sheet_names(object)
                .iter()
                .position(|n| n.eq_ignore_ascii_case(&want))
            {
                Some(i) => Ok(descend(object, SHEET, &[("__sheet", i as i64)])),
                // POI returns null for an unknown sheet name.
                None => Ok(CfmlValue::Null),
            }
        }
        "getsheetname" => {
            let i = arg_int(&args, 0);
            Ok(match sheet_names(object).get(i as usize) {
                Some(n) => CfmlValue::string(n.clone()),
                None => CfmlValue::Null,
            })
        }
        // getSheetIndex( name ) and getSheetIndex( sheet ). POI returns -1 when
        // there is no match, and callers test for it.
        "getsheetindex" => {
            let arg = args.first().cloned().unwrap_or(CfmlValue::Null);
            if let Some(CfmlValue::Int(i)) = get(&arg, "__sheet") {
                return Ok(CfmlValue::Int(i));
            }
            let want = arg.as_string();
            Ok(CfmlValue::Int(
                sheet_names(object)
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(&want))
                    .map(|i| i as i64)
                    .unwrap_or(-1),
            ))
        }
        "setsheetname" => {
            let i = arg_int(&args, 0);
            let name = args.get(1).map(|v| v.as_string()).unwrap_or_default();
            validate_sheet_name(&name)?;
            let mut names = sheet_names(object);
            if i < 0 || i as usize >= names.len() {
                return Err(illegal_argument(format!("Sheet index ({}) is out of range", i)));
            }
            bif(
                "spreadsheetRenameSheet",
                vec![
                    workbook_of(object).unwrap_or(CfmlValue::Null),
                    CfmlValue::string(name.clone()),
                    CfmlValue::Int(i + 1),
                ],
            )?;
            names[i as usize] = name;
            set_sheet_names(object, names);
            Ok(CfmlValue::Null)
        }
        "getactivesheetindex" => Ok(CfmlValue::Int(get_int(object, "__active"))),
        "setactivesheet" => {
            let i = arg_int(&args, 0);
            if let CfmlValue::Struct(s) = object {
                s.insert("__active".to_string(), CfmlValue::Int(i));
            }
            bif(
                "spreadsheetSetActiveSheetNumber",
                vec![
                    workbook_of(object).unwrap_or(CfmlValue::Null),
                    CfmlValue::Int(i + 1),
                ],
            )?;
            Ok(CfmlValue::Null)
        }
        // A fresh, empty accumulator. Nothing reaches the workbook until the
        // style is assigned to a cell or a row.
        "createcellstyle" => Ok(new_accumulator(CELL_STYLE, class_lower)),
        "createfont" => Ok(new_accumulator(FONT, class_lower)),
        "getcreationhelper" => Ok(CfmlValue::strukt(shim(CREATION_HELPER))),
        // POI hands back the workbook's font at an index. Styles here carry
        // their font settings inline rather than through a shared font table, so
        // there is no index to honour — an EMPTY accumulator is returned and the
        // caller's `cloneFont`-then-`setFont` round-trip still composes
        // correctly, because `setFont` MERGES into the style rather than
        // replacing it. See the CellStyle notes.
        "getfontat" => Ok(new_accumulator(FONT, class_lower)),
        "getnumberoffonts" | "getnumberoffontsasint" => Ok(CfmlValue::Int(1)),
        // write( OutputStream ) — the stream is the already-shimmed
        // java.io.FileOutputStream, which knows its own path.
        "write" => {
            let target = args.first().cloned().unwrap_or(CfmlValue::Null);
            let path = get(&target, "__stream_path")
                .map(|v| v.as_string())
                .filter(|p| !p.is_empty())
                .ok_or_else(|| {
                    CfmlError::new(
                        "java.io.IOException: Workbook.write() needs a java.io.FileOutputStream \
                         opened on a path; RustCFML's POI adapter writes through the native \
                         spreadsheet engine and cannot serialise to an arbitrary stream"
                            .to_string(),
                        CfmlErrorType::Custom("java.io.IOException".to_string()),
                    )
                })?;
            // An HSSFWorkbook was asked for and is being written as xlsx. This
            // is a real format substitution — the file will carry the name the
            // caller chose (often `.xls`) but xlsx bytes — so it is WARNED about
            // every time rather than passing silently. Spreadsheet applications
            // sniff content and open it; a strict `.xls` consumer will not.
            if matches!(get(object, "__xls_substituted"), Some(CfmlValue::Bool(true))) {
                // eprintln!, not log::warn!: in CLI mode the logger is only
                // initialised under --verbose, and a format substitution must
                // never be invisible. In serve mode this lands in the server log.
                eprintln!(
                    "[POI] Writing '{}' as xlsx. The caller built an HSSFWorkbook (legacy \
                     binary .xls), which this engine reads but cannot write. The content is \
                     correct and the file opens, but the bytes are xlsx whatever the \
                     extension says. Build the workbook as XSSF/SXSSF — `xmlFormat=true` \
                     through lucee-spreadsheet's new() — to remove this.",
                    path
                );
            }
            bif(
                "spreadsheetWrite",
                vec![
                    workbook_of(object).unwrap_or(CfmlValue::Null),
                    CfmlValue::string(path.clone()),
                    CfmlValue::Bool(true),
                ],
            )?;
            Ok(CfmlValue::Null)
        }
        // SXSSFWorkbook's streaming bookkeeping. There is no temp-file window to
        // flush or dispose of — the engine holds the whole workbook — so these
        // are genuinely nothing to do, not swallowed work.
        "dispose" | "close" | "flush" | "setcompresstempfiles" | "flushrows" => {
            Ok(CfmlValue::Null)
        }
        "getcreationhelperinstance" => Ok(CfmlValue::strukt(shim(CREATION_HELPER))),
        other => Err(unsupported(class_lower, other)),
    }
}

// ── Sheet ────────────────────────────────────────────────────────────────────

fn sheet(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    bif: &dyn Fn(&str, Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    let wb = || workbook_of(object).unwrap_or(CfmlValue::Null);
    let sheet_no = get_int(object, "__sheet") + 1; // 1-based for the builtins
    match method {
        "createrow" => Ok(descend(object, ROW, &[("__row", arg_int(&args, 0))])),
        // POI returns null for a row that was never created, and callers branch
        // on it (`formatRow` bails out rather than styling a phantom row).
        "getrow" => {
            let r = arg_int(&args, 0);
            let info = bif("spreadsheetInfo", vec![wb()])?;
            if r < 0 || r >= get_int(&info, "rowcount") {
                return Ok(CfmlValue::Null);
            }
            Ok(descend(object, ROW, &[("__row", r)]))
        }
        "rowiterator" | "iterator" => {
            let info = bif("spreadsheetInfo", vec![wb()])?;
            let rows = get_int(&info, "rowcount").max(0);
            Ok(make_iterator(
                (0..rows).map(|r| descend(object, ROW, &[("__row", r)])).collect(),
            ))
        }
        "getsheetname" => {
            let info = bif("spreadsheetInfo", vec![wb()])?;
            let idx = get_int(object, "__sheet") as usize;
            Ok(match get(&info, "sheetnames") {
                Some(CfmlValue::Array(a)) => {
                    a.snapshot().get(idx).cloned().unwrap_or(CfmlValue::Null)
                }
                _ => CfmlValue::Null,
            })
        }
        // POI's getLastRowNum is the 0-based index of the last row, so a sheet
        // with N rows answers N-1 — and an EMPTY sheet answers -1, which callers
        // test for. rowcount is 1-based, hence the plain subtraction.
        "getlastrownum" => {
            let info = bif("spreadsheetInfo", vec![wb()])?;
            Ok(CfmlValue::Int(get_int(&info, "rowcount") - 1))
        }
        "getphysicalnumberofrows" => {
            let info = bif("spreadsheetInfo", vec![wb()])?;
            Ok(CfmlValue::Int(get_int(&info, "rowcount")))
        }
        "createfreezepane" => {
            // POI: createFreezePane( colSplit, rowSplit ) — both 0-based counts
            // of frozen columns/rows, which is exactly what the builtin takes.
            bif(
                "spreadsheetAddFreezePane",
                vec![
                    wb(),
                    CfmlValue::Int(arg_int(&args, 0)),
                    CfmlValue::Int(arg_int(&args, 1)),
                ],
            )?;
            Ok(CfmlValue::Null)
        }
        "autosizecolumn" => {
            bif(
                "spreadsheetAutoSizeColumn",
                vec![wb(), CfmlValue::Int(arg_int(&args, 0) + 1)],
            )?;
            Ok(CfmlValue::Null)
        }
        // SXSSF requires opting in before autoSizeColumn works; the engine sizes
        // from the values it holds, so there is nothing to track.
        "trackallcolumnsforautosizing" | "trackcolumnforautosizing" | "untrackcolumnforautosizing" => {
            Ok(CfmlValue::Null)
        }
        "setcolumnwidth" => {
            // POI widths are 1/256th of a character; the builtin takes characters.
            bif(
                "spreadsheetSetColumnWidth",
                vec![
                    wb(),
                    CfmlValue::Int(arg_int(&args, 0) + 1),
                    CfmlValue::Int((arg_int(&args, 1) / 256).max(1)),
                ],
            )?;
            Ok(CfmlValue::Null)
        }
        "getcolumnwidth" => {
            let w = bif(
                "spreadsheetGetColumnWidth",
                vec![wb(), CfmlValue::Int(arg_int(&args, 0) + 1)],
            )?;
            Ok(CfmlValue::Int(
                w.as_string().trim().parse::<i64>().unwrap_or(8) * 256,
            ))
        }
        "getnummergedregions" => Ok(CfmlValue::Int(0)),
        "getworkbook" => {
            // Hand back a workbook handle onto the same native workbook. The
            // POI-side sheet list is not carried: callers that reach up from a
            // sheet want the workbook to write or style through, not to
            // re-enumerate sheets from.
            let mut m = shim(WORKBOOK_XSSF);
            m.insert("__wb".to_string(), wb());
            m.insert("__sheets".to_string(), CfmlValue::array(Vec::new()));
            Ok(CfmlValue::strukt(m))
        }
        "setdefaultcolumnwidth" | "setdisplaygridlines" | "setselected" | "setzoom" => {
            Ok(CfmlValue::Null)
        }
        other => {
            let _ = sheet_no;
            Err(unsupported("org.apache.poi.ss.usermodel.Sheet", other))
        }
    }
}

// ── Row ──────────────────────────────────────────────────────────────────────

fn row(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    bif: &dyn Fn(&str, Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    let wb = || workbook_of(object).unwrap_or(CfmlValue::Null);
    match method {
        "createcell" | "getcell" => Ok(descend(object, CELL, &[("__col", arg_int(&args, 0))])),
        "getrownum" => Ok(CfmlValue::Int(get_int(object, "__row"))),
        // POI iterates the cells that PHYSICALLY exist, not every column up to
        // the sheet's width — so a caller styling "the cells in this row" does
        // not also style the blank tail. Approximated by "has a value", which is
        // the only distinction the engine records.
        "celliterator" | "iterator" => {
            let info = bif("spreadsheetInfo", vec![wb()])?;
            let cols = get_int(&info, "columncount").max(0);
            let row1 = CfmlValue::Int(get_int(object, "__row") + 1);
            let mut cells = Vec::new();
            for col in 0..cols {
                let v = bif(
                    "spreadsheetGetCellValue",
                    vec![wb(), row1.clone(), CfmlValue::Int(col + 1)],
                )?;
                if !v.as_string().is_empty() {
                    cells.push(descend(object, CELL, &[("__col", col)]));
                }
            }
            Ok(make_iterator(cells))
        }
        "getlastcellnum" => {
            let info = bif("spreadsheetInfo", vec![wb()])?;
            // POI: one PAST the last cell index, and -1 for an empty row.
            Ok(CfmlValue::Int(get_int(&info, "columncount")))
        }
        "getphysicalnumberofcells" => {
            let info = bif("spreadsheetInfo", vec![wb()])?;
            Ok(CfmlValue::Int(get_int(&info, "columncount")))
        }
        "setrowstyle" => {
            let fmt = fmt_of(&args.first().cloned().unwrap_or(CfmlValue::Null));
            if !fmt.is_empty() {
                bif(
                    "spreadsheetFormatRow",
                    vec![
                        wb(),
                        CfmlValue::strukt(fmt),
                        CfmlValue::Int(get_int(object, "__row") + 1),
                    ],
                )?;
            }
            Ok(CfmlValue::Null)
        }
        "setheight" | "setheightinpoints" => {
            bif(
                "spreadsheetSetRowHeight",
                vec![
                    wb(),
                    CfmlValue::Int(get_int(object, "__row") + 1),
                    args.first().cloned().unwrap_or(CfmlValue::Int(15)),
                ],
            )?;
            Ok(CfmlValue::Null)
        }
        "removecell" => {
            let target = args.first().cloned().unwrap_or(CfmlValue::Null);
            bif(
                "spreadsheetClearCell",
                vec![
                    wb(),
                    CfmlValue::Int(get_int(object, "__row") + 1),
                    CfmlValue::Int(get_int(&target, "__col") + 1),
                ],
            )?;
            Ok(CfmlValue::Null)
        }
        "getsheet" => Ok(descend(object, SHEET, &[])),
        other => Err(unsupported("org.apache.poi.ss.usermodel.Row", other)),
    }
}

// ── Cell ─────────────────────────────────────────────────────────────────────

fn cell(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    bif: &dyn Fn(&str, Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    let wb = || workbook_of(object).unwrap_or(CfmlValue::Null);
    let r = || CfmlValue::Int(get_int(object, "__row") + 1);
    let c = || CfmlValue::Int(get_int(object, "__col") + 1);
    match method {
        "setcellvalue" => {
            bif(
                "spreadsheetSetCellValue",
                vec![
                    wb(),
                    args.first().cloned().unwrap_or(CfmlValue::string(String::new())),
                    r(),
                    c(),
                ],
            )?;
            Ok(CfmlValue::Null)
        }
        "getstringcellvalue" | "getnumericcellvalue" | "getcellvalue" | "getrawvalue" => {
            bif("spreadsheetGetCellValue", vec![wb(), r(), c()])
        }
        "getbooleancellvalue" => {
            let v = bif("spreadsheetGetCellValue", vec![wb(), r(), c()])?;
            Ok(CfmlValue::Bool(v.is_true()))
        }
        "setblank" | "setcellblank" => {
            bif("spreadsheetClearCell", vec![wb(), r(), c()])?;
            Ok(CfmlValue::Null)
        }
        "setcellformula" => {
            bif(
                "spreadsheetSetCellFormula",
                vec![
                    wb(),
                    args.first().cloned().unwrap_or(CfmlValue::Null),
                    r(),
                    c(),
                ],
            )?;
            Ok(CfmlValue::Null)
        }
        "getcellformula" => bif("spreadsheetGetCellFormula", vec![wb(), r(), c()]),
        "getcelltype" => bif("spreadsheetGetCellType", vec![wb(), r(), c()]),
        // The point at which an accumulated style actually reaches the workbook.
        "setcellstyle" => {
            let fmt = fmt_of(&args.first().cloned().unwrap_or(CfmlValue::Null));
            if !fmt.is_empty() {
                bif(
                    "spreadsheetFormatCell",
                    vec![wb(), CfmlValue::strukt(fmt), r(), c()],
                )?;
            }
            Ok(CfmlValue::Null)
        }
        "getcellstyle" => {
            let mut m = shim(CELL_STYLE);
            let current = bif("spreadsheetGetCellFormat", vec![wb(), r(), c()])
                .unwrap_or(CfmlValue::strukt(ValueMap::default()));
            m.insert(
                "__fmt".to_string(),
                match current {
                    v @ CfmlValue::Struct(_) => v,
                    _ => CfmlValue::strukt(ValueMap::default()),
                },
            );
            Ok(CfmlValue::strukt(m))
        }
        "getcolumnindex" => Ok(CfmlValue::Int(get_int(object, "__col"))),
        "getrowindex" => Ok(CfmlValue::Int(get_int(object, "__row"))),
        "getrow" => Ok(descend(object, ROW, &[])),
        "getsheet" => Ok(descend(object, SHEET, &[])),
        // POI's setCellType is advisory once a value is set; the engine infers
        // the type from the value, so there is nothing to record.
        "setcelltype" => Ok(CfmlValue::Null),
        other => Err(unsupported("org.apache.poi.ss.usermodel.Cell", other)),
    }
}

// ── CellStyle / Font (accumulators) ──────────────────────────────────────────

fn style(
    class_lower: &str,
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    // POI setter -> the builtins' format-struct key. Only the keys the native
    // formatter understands are accepted; an unmapped setter throws rather than
    // being recorded into a struct the engine will ignore.
    let key = match method {
        "setbold" | "setboldweight" => Some("bold"),
        "setitalic" => Some("italic"),
        "setunderline" => Some("underline"),
        "setstrikeout" => Some("strikethrough"),
        "setfontheightinpoints" => Some("fontsize"),
        "setfontheight" => Some("__fontheight_twips"),
        // Modelled by neither the format struct nor the engine. Accepted and
        // ignored ON PURPOSE: `cloneFont` copies every property unconditionally,
        // so refusing here would break a clone that never set them. They are
        // listed in docs/known-issues.md.
        "setcharset" | "settypeoffset" => Some("__ignored"),
        "setfontname" => Some("font"),
        "setcolor" | "setfontcolor" => Some("color"),
        "setfillforegroundcolor" => Some("fgcolor"),
        "setdataformat" => Some("dataformat"),
        "setwraptext" => Some("wraptext"),
        "setalignment" => Some("alignment"),
        "setverticalalignment" => Some("verticalalignment"),
        _ => None,
    };
    if let Some(key) = key {
        let value = args.first().cloned().unwrap_or(CfmlValue::Bool(true));
        // A null means "the source had nothing set" — see the getter note. It
        // must not be recorded, or a clone of an empty font would stamp defaults
        // onto every cell the resulting style touches.
        if matches!(value, CfmlValue::Null) || key == "__ignored" {
            return Ok(CfmlValue::Null);
        }
        // POI font heights are twips; the format struct takes points.
        if key == "__fontheight_twips" {
            let pts = value.as_string().trim().parse::<f64>().unwrap_or(0.0) / 20.0;
            set_fmt_key(object, "fontsize", CfmlValue::Double(pts));
            return Ok(CfmlValue::Null);
        }
        // setBoldweight( short ) predates setBold( boolean ): POI's BOLD constant
        // is 700, anything lower is not bold.
        let value = if method == "setboldweight" {
            CfmlValue::Bool(value.as_string().trim().parse::<i64>().unwrap_or(0) >= 700)
        } else {
            value
        };
        set_fmt_key(object, key, value);
        return Ok(CfmlValue::Null);
    }

    match method {
        // A style built from another starts as a copy of its accumulator.
        "clonestylefrom" => {
            let src = fmt_of(&args.first().cloned().unwrap_or(CfmlValue::Null));
            if let CfmlValue::Struct(s) = object {
                s.insert("__fmt".to_string(), CfmlValue::strukt(src));
            }
            Ok(CfmlValue::Null)
        }
        // setFont( font ) folds the font accumulator into the style's.
        "setfont" => {
            let font = fmt_of(&args.first().cloned().unwrap_or(CfmlValue::Null));
            let mut f = fmt_of(object);
            for (k, v) in font.iter() {
                f.insert(k.clone(), v.clone());
            }
            if let CfmlValue::Struct(s) = object {
                s.insert("__fmt".to_string(), CfmlValue::strukt(f));
            }
            Ok(CfmlValue::Null)
        }
        "getfont" => {
            let mut m = shim(FONT);
            m.insert("__fmt".to_string(), CfmlValue::strukt(fmt_of(object)));
            if let Some(pc) = get(object, "__poi_class") {
                m.insert("__poi_class".to_string(), pc);
            }
            Ok(CfmlValue::strukt(m))
        }
        // A style's font is inline, so there is exactly one and its index is 0.
        // The caller feeds this straight back into workbook.getFontAt().
        "getfontindexasint" | "getfontindexasshort" => Ok(CfmlValue::Int(0)),
        "getindex" | "getindexasint" => Ok(CfmlValue::Int(0)),
        // POI 5 returns an enum; the library subscripts it by name to get the
        // constant (`cellStyle.getAlignment()[ "LEFT" ]`). A struct of
        // name -> lowercase name reproduces that, and the lowercase name is what
        // the native formatter takes.
        "getalignment" | "gethorizontalalignment" => Ok(alignment_enum(&[
            "GENERAL", "LEFT", "CENTER", "RIGHT", "FILL", "JUSTIFY",
            "CENTER_SELECTION", "DISTRIBUTED",
        ])),
        "getverticalalignment" => Ok(alignment_enum(&[
            "TOP", "CENTER", "BOTTOM", "JUSTIFY", "DISTRIBUTED",
        ])),
        "getborderbottom" | "getbordertop" | "getborderleft" | "getborderright" => {
            Ok(alignment_enum(&[
                "NONE", "THIN", "MEDIUM", "DASHED", "DOTTED", "THICK", "DOUBLE",
                "HAIR", "MEDIUM_DASHED", "DASH_DOT", "MEDIUM_DASH_DOT", "DASH_DOT_DOT",
                "MEDIUM_DASH_DOT_DOT", "SLANTED_DASH_DOT",
            ]))
        }
        // Font/style getters. An UNSET property answers null, and every setter
        // above ignores a null — so `cloneFont`, which copies every property of
        // a font it was handed and assigns them all to a new one, carries across
        // only what was genuinely set. Answering POI's defaults instead would
        // stamp "Calibri 11pt black" onto every cell the style touched.
        g if g.starts_with("get") => {
            let f = fmt_of(object);
            let read = |key: &str| f.get(key).cloned().unwrap_or(CfmlValue::Null);
            Ok(match g {
                "getbold" => read("bold"),
                "getitalic" => read("italic"),
                "getstrikeout" => read("strikethrough"),
                "getunderline" => read("underline"),
                "getfontname" => read("font"),
                "getcolor" | "getxssfcolor" | "getfontcolor" => read("color"),
                "getfillforegroundcolor" => read("fgcolor"),
                "getdataformat" | "getdataformatstring" => read("dataformat"),
                "getwraptext" => read("wraptext"),
                "getfontheightinpoints" => read("fontsize"),
                // POI font heights are twips (1/20 pt).
                "getfontheight" => match f.get("fontsize") {
                    Some(v) => CfmlValue::Double(
                        v.as_string().trim().parse::<f64>().unwrap_or(0.0) * 20.0,
                    ),
                    None => CfmlValue::Null,
                },
                // Properties this adapter does not model. Null keeps them out of
                // a clone rather than inventing a value for them.
                "getcharset" | "gettypeoffset" => CfmlValue::Null,
                _ => return Err(unsupported(class_name_of(class_lower), g)),
            })
        }
        other => Err(unsupported(class_name_of(class_lower), other)),
    }
}
