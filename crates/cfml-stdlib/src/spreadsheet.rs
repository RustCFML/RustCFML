//! Native CFML spreadsheet support — the `Spreadsheet*` BIFs, the fluent
//! `Spreadsheet()` builder, and (later) `<cfspreadsheet>`.
//!
//! A workbook is a first-class, mutable CFML object, modelled exactly like the
//! image object (see [`crate::image`]): a [`CfmlValue::NativeObject`] wrapping
//! [`CfmlSpreadsheet`], which implements [`CfmlNative`]. That gives both call
//! forms for free:
//!
//! * **member / fluent form** — `wb.setCellValue(1,1,"x").formatRow(1,{bold:true})`
//!   — dispatched by the VM straight to [`CfmlSpreadsheet::call_method`]. Mutating
//!   methods return the *same* workbook handle (via a `Weak` self-reference set at
//!   construction) so calls chain; terminal reads return data.
//! * **function form** — `SpreadsheetSetCellValue(wb,"x",1,1)` — a plain builtin
//!   that locks the same `Arc` and forwards to `call_method`. Because
//!   `NativeObject` is a shared handle, the mutation is visible through the
//!   caller's variable, exactly like Lucee/ACF.
//!
//! Backed by the pure-Rust `umya-spreadsheet` (POI-model open→mutate→save with
//! styles/charts/images preserved) for `.xlsx`, and `calamine` for legacy
//! `.xls`/`.xlsb` reads. NATIVE/SERVER-ONLY — compiled behind the `spreadsheet`
//! feature and deliberately excluded from the wasm builds.
//!
//! Addressing is **1-based** row/column at the CFML boundary (CFML convention),
//! converted to umya's 1-based `(col,row)` tuple internally.

use cfml_common::dynamic::{CfmlNative, CfmlQuery, CfmlStruct, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use std::sync::{Arc, RwLock, Weak};
use umya_spreadsheet::structs::drawing::spreadsheet::MarkerType;
use umya_spreadsheet::{
    Break, Chart, ChartType, Color, ColorScale, Comment, ConditionalFormatting,
    ConditionalFormattingRule, ConditionalFormatValueObject, ConditionalFormatValueObjectValues,
    ConditionalFormatValues, ConditionalFormattingOperatorValues, Coordinate, DataValidation,
    DataValidations, DataValidationOperatorValues, DataValidationValues, Formula, Hyperlink,
    HorizontalAlignmentValues, Image as XlsxImage, NumberingFormat, OddFooter, OddHeader,
    OrientationValues, Pane, PaneStateValues, PaneValues, SheetView, Style,
    VerticalAlignmentValues, Workbook,
};

/// Workbook file format. Drives `SpreadsheetNew`'s `xmlformat` flag and the
/// write path (umya writes `.xlsx`; legacy `.xls` write is unsupported).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbookFormat {
    /// Office Open XML (`.xlsx`) — the default and only writable format.
    Xlsx,
    /// Legacy BIFF (`.xls`) — readable via calamine, not writable by umya.
    Xls,
}

/// A live, mutable CFML workbook object.
pub struct CfmlSpreadsheet {
    /// The underlying umya workbook (mutable in-memory model).
    book: Workbook,
    /// 0-based index of the currently-active sheet. Most operations target it
    /// unless a sheet is selected first.
    active_sheet: usize,
    /// On-disk format this workbook represents.
    format: WorkbookFormat,
    /// `Weak` self-reference, set at [`CfmlSpreadsheet::into_value`] time via
    /// `Arc::new_cyclic`, so mutating methods can return the *same* handle for
    /// fluent chaining without cloning the (potentially large) workbook.
    self_ref: Option<Weak<RwLock<CfmlSpreadsheet>>>,
}

impl std::fmt::Debug for CfmlSpreadsheet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CfmlSpreadsheet")
            .field("active_sheet", &self.active_sheet)
            .field("format", &self.format)
            .field("sheets", &self.book.sheet_count())
            .finish()
    }
}

impl CfmlSpreadsheet {
    /// Wrap an umya workbook + format into the shared `NativeObject` handle,
    /// stamping the `Weak` self-reference so fluent mutators can return `this`.
    pub fn into_value(mut self, format: WorkbookFormat) -> CfmlValue {
        self.format = format;
        let arc: Arc<RwLock<CfmlSpreadsheet>> = Arc::new_cyclic(|weak| {
            self.self_ref = Some(weak.clone());
            RwLock::new(self)
        });
        CfmlValue::NativeObject(arc)
    }

    /// A fresh `.xlsx` workbook (one default sheet named `sheetName`, or
    /// "Sheet1"). The umya default file already has a "Sheet1"; rename it when a
    /// name is supplied.
    pub fn new_xlsx(sheet_name: Option<&str>) -> CfmlSpreadsheet {
        let mut book = umya_spreadsheet::new_file();
        if let Some(name) = sheet_name {
            if let Ok(ws) = book.sheet_mut(0) {
                ws.set_name(name);
            }
        }
        CfmlSpreadsheet { book, active_sheet: 0, format: WorkbookFormat::Xlsx, self_ref: None }
    }

    /// Return the fluent self-handle (same `Arc`) for chaining, or `Null` if the
    /// self-reference is somehow gone (never expected in practice).
    fn this(&self) -> CfmlValue {
        match self.self_ref.as_ref().and_then(|w| w.upgrade()) {
            Some(arc) => CfmlValue::NativeObject(arc),
            None => CfmlValue::Null,
        }
    }

    /// The active worksheet, mutably. Errors if the index is somehow invalid.
    fn active_ws_mut(&mut self) -> Result<&mut umya_spreadsheet::Worksheet, CfmlError> {
        self.book
            .sheet_mut(self.active_sheet)
            .map_err(|_| CfmlError::runtime("Spreadsheet has no active sheet".to_string()))
    }

    // ---- primitive operations (shared by member + function forms) ----------

    fn set_cell_value(&mut self, value: &CfmlValue, row: u32, col: u32) -> Result<(), CfmlError> {
        let ws = self.active_ws_mut()?;
        let cell = ws.cell_mut((col, row));
        match value {
            CfmlValue::Int(i) => { cell.set_value_number(*i as f64); }
            CfmlValue::Double(d) | CfmlValue::TimeSpan(d) => { cell.set_value_number(*d); }
            CfmlValue::Bool(b) => { cell.set_value_bool(*b); }
            CfmlValue::Null => { cell.set_value(""); }
            other => { cell.set_value(other.as_string()); }
        }
        Ok(())
    }

    fn get_cell_value(&self, row: u32, col: u32) -> CfmlValue {
        match self.book.sheet(self.active_sheet) {
            Ok(ws) => CfmlValue::string(ws.value((col, row))),
            Err(_) => CfmlValue::string(String::new()),
        }
    }

    fn create_sheet(&mut self, name: &str) -> Result<(), CfmlError> {
        self.book
            .new_sheet(name)
            .map(|_| ())
            .map_err(|e| CfmlError::runtime(format!("Cannot create sheet [{}]: {:?}", name, e)))
    }

    fn rename_sheet(&mut self, name: &str, sheet_number: usize) -> Result<(), CfmlError> {
        // CFML sheet numbers are 1-based.
        let idx = sheet_number.saturating_sub(1);
        let ws = self
            .book
            .sheet_mut(idx)
            .map_err(|_| CfmlError::runtime(format!("No sheet at index {}", sheet_number)))?;
        ws.set_name(name);
        Ok(())
    }

    fn row_count(&self) -> i64 {
        self.book
            .sheet(self.active_sheet)
            .map(|ws| ws.highest_row() as i64)
            .unwrap_or(0)
    }

    fn column_count(&self) -> i64 {
        self.book
            .sheet(self.active_sheet)
            .map(|ws| ws.highest_column() as i64)
            .unwrap_or(0)
    }

    fn write_to(&self, path: &str, password: Option<&str>) -> Result<(), CfmlError> {
        if self.format == WorkbookFormat::Xls {
            return Err(CfmlError::runtime(
                "Writing legacy .xls (BIFF) is not supported; use .xlsx".to_string(),
            ));
        }
        let p = std::path::Path::new(path);
        match password {
            Some(pw) if !pw.is_empty() => {
                umya_spreadsheet::writer::xlsx::write_with_password(&self.book, p, pw)
                    .map_err(|e| CfmlError::runtime(format!("Unable to write protected spreadsheet [{}]: {:?}", path, e)))
            }
            _ => umya_spreadsheet::writer::xlsx::write(&self.book, p)
                .map_err(|e| CfmlError::runtime(format!("Unable to write spreadsheet [{}]: {:?}", path, e))),
        }
    }

    fn info_struct(&self) -> CfmlValue {
        use cfml_common::dynamic::ValueMap;
        let mut m = ValueMap::default();
        m.insert("sheets".to_string(), CfmlValue::Int(self.book.sheet_count() as i64));
        let names: Vec<CfmlValue> = self
            .book
            .sheet_collection()
            .iter()
            .map(|ws| CfmlValue::string(ws.name().to_string()))
            .collect();
        m.insert("sheetnames".to_string(), CfmlValue::array(names));
        m.insert("rowcount".to_string(), CfmlValue::Int(self.row_count()));
        m.insert("columncount".to_string(), CfmlValue::Int(self.column_count()));
        m.insert("format".to_string(), CfmlValue::string(match self.format {
            WorkbookFormat::Xlsx => "xlsx",
            WorkbookFormat::Xls => "xls",
        }.to_string()));
        CfmlValue::strukt(m)
    }

    /// Read an existing `.xlsx` from disk into a fresh, mutable workbook — the
    /// POI-model round-trip entry point (styles/charts/images preserved).
    pub fn read_file(path: &str) -> Result<CfmlSpreadsheet, CfmlError> {
        let book = umya_spreadsheet::reader::xlsx::read(std::path::Path::new(path))
            .map_err(|e| CfmlError::runtime(format!("Unable to read spreadsheet [{}]: {:?}", path, e)))?;
        Ok(CfmlSpreadsheet { book, active_sheet: 0, format: WorkbookFormat::Xlsx, self_ref: None })
    }

    /// Serialise the workbook to an in-memory `.xlsx` byte buffer.
    fn to_binary(&self) -> Result<Vec<u8>, CfmlError> {
        let mut buf = std::io::Cursor::new(Vec::new());
        umya_spreadsheet::writer::xlsx::write_writer(&self.book, &mut buf)
            .map_err(|e| CfmlError::runtime(format!("Unable to serialise spreadsheet: {:?}", e)))?;
        Ok(buf.into_inner())
    }

    fn set_active_sheet_by_name(&mut self, name: &str) -> Result<(), CfmlError> {
        let idx = self
            .book
            .sheet_collection()
            .iter()
            .position(|ws| ws.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| CfmlError::runtime(format!("No sheet named [{}]", name)))?;
        self.active_sheet = idx;
        self.book.set_active_sheet(idx as u32);
        Ok(())
    }

    fn set_active_sheet_number(&mut self, num: usize) -> Result<(), CfmlError> {
        let idx = num.saturating_sub(1);
        if idx >= self.book.sheet_count() {
            return Err(CfmlError::runtime(format!("No sheet at index {}", num)));
        }
        self.active_sheet = idx;
        self.book.set_active_sheet(idx as u32);
        Ok(())
    }

    /// Write cells across a row from `start_col`; optionally insert (shift down).
    fn add_row(&mut self, data: &CfmlValue, row: u32, start_col: u32, insert: bool, delimiter: &str) -> Result<(), CfmlError> {
        let cells = value_to_cells(data, delimiter);
        if insert {
            self.active_ws_mut()?.insert_new_row(row, 1);
        }
        for (i, cell) in cells.iter().enumerate() {
            self.set_cell_value(cell, row, start_col + i as u32)?;
        }
        Ok(())
    }

    /// Append multiple rows from a query or an array (of arrays/lists/structs).
    fn add_rows(&mut self, data: &CfmlValue, start_row: Option<u32>, start_col: u32, include_headers: bool) -> Result<(), CfmlError> {
        let mut row = start_row.unwrap_or_else(|| (self.row_count() as u32) + 1);
        match data {
            CfmlValue::Query(q) => {
                let cols = q.columns();
                if include_headers {
                    for (i, name) in cols.iter().enumerate() {
                        self.set_cell_value(&CfmlValue::string(name.clone()), row, start_col + i as u32)?;
                    }
                    row += 1;
                }
                for r in q.rows() {
                    for (i, name) in cols.iter().enumerate() {
                        let cell = r.get(name).cloned().unwrap_or(CfmlValue::Null);
                        self.set_cell_value(&cell, row, start_col + i as u32)?;
                    }
                    row += 1;
                }
            }
            CfmlValue::Array(a) => {
                for item in a.snapshot() {
                    let cells = value_to_cells(&item, ",");
                    for (i, cell) in cells.iter().enumerate() {
                        self.set_cell_value(cell, row, start_col + i as u32)?;
                    }
                    row += 1;
                }
            }
            other => {
                return Err(CfmlError::runtime(format!(
                    "addRows expects a query or array, got {}",
                    other.type_name()
                )));
            }
        }
        Ok(())
    }

    /// Write values down a column from `start_row`; optionally insert (shift right).
    fn add_column(&mut self, data: &CfmlValue, start_row: u32, start_col: u32, insert: bool, delimiter: &str) -> Result<(), CfmlError> {
        let cells = value_to_cells(data, delimiter);
        if insert {
            self.active_ws_mut()?.insert_new_column_by_index(start_col, 1);
        }
        for (i, cell) in cells.iter().enumerate() {
            self.set_cell_value(cell, start_row + i as u32, start_col)?;
        }
        Ok(())
    }

    fn format_cell(&mut self, fmt: &CfmlStruct, row: u32, col: u32) -> Result<(), CfmlError> {
        let ws = self.active_ws_mut()?;
        apply_format(ws.style_mut((col, row)), fmt);
        Ok(())
    }

    fn format_row(&mut self, fmt: &CfmlStruct, row: u32) -> Result<(), CfmlError> {
        let last_col = (self.column_count() as u32).max(1);
        let ws = self.active_ws_mut()?;
        for col in 1..=last_col {
            apply_format(ws.style_mut((col, row)), fmt);
        }
        Ok(())
    }

    fn format_column(&mut self, fmt: &CfmlStruct, col: u32) -> Result<(), CfmlError> {
        let last_row = (self.row_count() as u32).max(1);
        let ws = self.active_ws_mut()?;
        for row in 1..=last_row {
            apply_format(ws.style_mut((col, row)), fmt);
        }
        Ok(())
    }

    fn format_cell_range(&mut self, fmt: &CfmlStruct, sr: u32, sc: u32, er: u32, ec: u32) -> Result<(), CfmlError> {
        let ws = self.active_ws_mut()?;
        for row in sr..=er {
            for col in sc..=ec {
                apply_format(ws.style_mut((col, row)), fmt);
            }
        }
        Ok(())
    }

    fn merge_cells(&mut self, sr: u32, sc: u32, er: u32, ec: u32) -> Result<(), CfmlError> {
        let range = format!("{}:{}", a1(sc, sr), a1(ec, er));
        self.active_ws_mut()?.add_merge_cells(range);
        Ok(())
    }

    /// Freeze `freeze_col` leftmost columns and `freeze_row` top rows. NB: umya
    /// maps `horizontal_split → xSplit` (columns) and `vertical_split → ySplit`
    /// (rows) — counterintuitive, but that's the OOXML attribute mapping.
    fn add_freeze_pane(&mut self, freeze_col: u32, freeze_row: u32) -> Result<(), CfmlError> {
        let mut pane = Pane::default();
        pane.set_state(PaneStateValues::Frozen);
        pane.set_horizontal_split(freeze_col as f64); // xSplit = columns frozen
        pane.set_vertical_split(freeze_row as f64); // ySplit = rows frozen
        let mut tl = Coordinate::default();
        tl.set_col_num(freeze_col + 1);
        tl.set_row_num(freeze_row + 1);
        pane.set_top_left_cell(tl);
        pane.set_active_pane(PaneValues::BottomRight);
        let views = self.active_ws_mut()?.sheet_views_mut();
        if views.sheet_view_list_mut().is_empty() {
            views.add_sheet_view_list_mut(SheetView::default());
        }
        views.sheet_view_list_mut()[0].set_pane(pane);
        Ok(())
    }

    fn auto_size_column(&mut self, col: u32) -> Result<(), CfmlError> {
        let dim = self.active_ws_mut()?.column_dimension_by_number_mut(col);
        dim.set_auto_width(true);
        dim.set_best_fit(true);
        Ok(())
    }

    fn set_column_width(&mut self, col: u32, width: f64) -> Result<(), CfmlError> {
        self.active_ws_mut()?.column_dimension_by_number_mut(col).set_width(width);
        Ok(())
    }

    fn set_row_height(&mut self, row: u32, height: f64) -> Result<(), CfmlError> {
        self.active_ws_mut()?.row_dimension_mut(row).set_height(height);
        Ok(())
    }

    // ---- deletes / shifts --------------------------------------------------

    fn delete_row(&mut self, row: u32) -> Result<(), CfmlError> {
        self.active_ws_mut()?.remove_row(row, 1);
        Ok(())
    }

    fn delete_rows(&mut self, range: &str) -> Result<(), CfmlError> {
        // Remove from the bottom up so earlier indices stay valid.
        let mut rows = parse_range_list(range);
        rows.sort_unstable();
        rows.dedup();
        for r in rows.into_iter().rev() {
            self.active_ws_mut()?.remove_row(r, 1);
        }
        Ok(())
    }

    fn delete_column(&mut self, col: u32) -> Result<(), CfmlError> {
        self.active_ws_mut()?.remove_column_by_index(col, 1);
        Ok(())
    }

    fn delete_columns(&mut self, range: &str) -> Result<(), CfmlError> {
        let mut cols = parse_range_list(range);
        cols.sort_unstable();
        cols.dedup();
        for c in cols.into_iter().rev() {
            self.active_ws_mut()?.remove_column_by_index(c, 1);
        }
        Ok(())
    }

    fn shift_rows(&mut self, start: u32, end: u32, offset: i32) -> Result<(), CfmlError> {
        let last_col = (self.column_count() as u32).max(1);
        let range = format!("{}:{}", a1(1, start), a1(last_col, end));
        self.active_ws_mut()?.move_range(&range, offset, 0);
        Ok(())
    }

    fn shift_columns(&mut self, start: u32, end: u32, offset: i32) -> Result<(), CfmlError> {
        let last_row = (self.row_count() as u32).max(1);
        let range = format!("{}:{}", a1(start, 1), a1(end, last_row));
        self.active_ws_mut()?.move_range(&range, 0, offset);
        Ok(())
    }

    // ---- formulas / cell type / clear -------------------------------------

    fn set_cell_formula(&mut self, formula: &str, row: u32, col: u32) -> Result<(), CfmlError> {
        self.active_ws_mut()?.cell_mut((col, row)).set_formula(formula);
        Ok(())
    }

    fn get_cell_formula(&self, row: u32, col: u32) -> CfmlValue {
        match self.book.sheet(self.active_sheet) {
            Ok(ws) => match ws.cell((col, row)) {
                Some(c) => CfmlValue::string(c.formula().to_string()),
                None => CfmlValue::string(String::new()),
            },
            Err(_) => CfmlValue::string(String::new()),
        }
    }

    fn get_cell_type(&self, row: u32, col: u32) -> CfmlValue {
        match self.book.sheet(self.active_sheet) {
            Ok(ws) => match ws.cell((col, row)) {
                Some(c) => CfmlValue::string(c.data_type().to_string()),
                None => CfmlValue::string("undefined".to_string()),
            },
            Err(_) => CfmlValue::string("undefined".to_string()),
        }
    }

    fn clear_cell(&mut self, row: u32, col: u32) -> Result<(), CfmlError> {
        self.active_ws_mut()?.remove_cell((col, row));
        Ok(())
    }

    fn clear_cell_range(&mut self, sr: u32, sc: u32, er: u32, ec: u32) -> Result<(), CfmlError> {
        let ws = self.active_ws_mut()?;
        for row in sr..=er {
            for col in sc..=ec {
                ws.remove_cell((col, row));
            }
        }
        Ok(())
    }

    fn set_cell_range_value(&mut self, value: &CfmlValue, sr: u32, sc: u32, er: u32, ec: u32) -> Result<(), CfmlError> {
        for row in sr..=er {
            for col in sc..=ec {
                self.set_cell_value(value, row, col)?;
            }
        }
        Ok(())
    }

    // ---- hidden rows / columns --------------------------------------------

    fn set_column_hidden(&mut self, col: u32, hidden: bool) -> Result<(), CfmlError> {
        self.active_ws_mut()?.column_dimension_by_number_mut(col).set_hidden(hidden);
        Ok(())
    }

    fn set_row_hidden(&mut self, row: u32, hidden: bool) -> Result<(), CfmlError> {
        self.active_ws_mut()?.row_dimension_mut(row).set_hidden(hidden);
        Ok(())
    }

    fn is_column_hidden(&self, col: u32) -> bool {
        // Falls back to false when the column has no explicit dimension.
        matches!(self.book.sheet(self.active_sheet), Ok(ws)
            if ws.column_dimensions().iter().any(|d| d.col_num() == col && d.hidden()))
    }

    fn is_row_hidden(&self, row: u32) -> bool {
        matches!(self.book.sheet(self.active_sheet), Ok(ws)
            if ws.row_dimensions().iter().any(|d| d.row_num() == row && d.hidden()))
    }

    // ---- comments / hyperlinks / autofilter -------------------------------

    fn set_cell_comment(&mut self, text: &str, author: &str, row: u32, col: u32) -> Result<(), CfmlError> {
        let mut c = Comment::default();
        c.new_comment((col, row));
        if !author.is_empty() {
            c.set_author(author);
        }
        c.set_text_string(text);
        self.active_ws_mut()?.add_comments(c);
        Ok(())
    }

    fn set_cell_hyperlink(&mut self, link: &str, row: u32, col: u32, tooltip: &str, cell_value: Option<&CfmlValue>) -> Result<(), CfmlError> {
        if let Some(v) = cell_value {
            self.set_cell_value(v, row, col)?;
        } else {
            // Default the visible text to the link if the cell is empty.
            if self.get_cell_value(row, col).as_string().is_empty() {
                self.set_cell_value(&CfmlValue::string(link.to_string()), row, col)?;
            }
        }
        let hl = self.active_ws_mut()?.cell_mut((col, row)).hyperlink_mut();
        hl.set_url(link);
        if !tooltip.is_empty() {
            hl.set_tooltip(tooltip);
        }
        Ok(())
    }

    fn add_autofilter(&mut self, range: &str) -> Result<(), CfmlError> {
        self.active_ws_mut()?.set_auto_filter(range);
        Ok(())
    }

    fn add_info(&mut self, info: &CfmlStruct) -> Result<(), CfmlError> {
        let props = self.book.properties_mut();
        if let Some(v) = info.get_ci("title") { props.set_title(v.as_string()); }
        if let Some(v) = info.get_ci("subject") { props.set_subject(v.as_string()); }
        if let Some(v) = info.get_ci("author") { props.set_creator(v.as_string()); }
        if let Some(v) = info.get_ci("creator") { props.set_creator(v.as_string()); }
        if let Some(v) = info.get_ci("category") { props.set_category(v.as_string()); }
        if let Some(v) = info.get_ci("keywords") { props.set_keywords(v.as_string()); }
        if let Some(v) = info.get_ci("comments") { props.set_description(v.as_string()); }
        if let Some(v) = info.get_ci("manager") { props.set_manager(v.as_string()); }
        if let Some(v) = info.get_ci("company") { props.set_company(v.as_string()); }
        Ok(())
    }

    // ---- images / charts ---------------------------------------------------

    fn add_image(&mut self, path: &str, anchor: &str) -> Result<(), CfmlError> {
        if path.is_empty() {
            return Err(CfmlError::runtime("addImage: a file path is required".to_string()));
        }
        let mut marker = MarkerType::default();
        marker.set_coordinate(anchor_to_a1(anchor));
        let mut img = XlsxImage::default();
        img.new_image(path, marker);
        self.active_ws_mut()?.add_image(img);
        Ok(())
    }

    fn add_chart(&mut self, chart_type: &str, series: &[String], from_a1: &str, to_a1: &str, title: &str) -> Result<(), CfmlError> {
        let ct = match chart_type.to_ascii_lowercase().as_str() {
            "bar" => ChartType::BarChart,
            "bar3d" => ChartType::Bar3DChart,
            "pie" => ChartType::PieChart,
            "pie3d" => ChartType::Pie3DChart,
            "doughnut" => ChartType::DoughnutChart,
            "area" => ChartType::AreaChart,
            "line3d" => ChartType::Line3DChart,
            _ => ChartType::LineChart,
        };
        let mut from = MarkerType::default();
        from.set_coordinate(from_a1);
        let mut to = MarkerType::default();
        to.set_coordinate(to_a1);
        let refs: Vec<&str> = series.iter().map(|s| s.as_str()).collect();
        let mut chart = Chart::default();
        chart.new_chart(&ct, from, to, refs);
        if !title.is_empty() {
            chart.set_title(title);
        }
        self.active_ws_mut()?.add_chart(chart);
        Ok(())
    }

    // ---- data interchange --------------------------------------------------

    /// The active sheet as a CFML query — row 1 supplies column names.
    fn to_query(&self) -> CfmlValue {
        let ws = match self.book.sheet(self.active_sheet) {
            Ok(ws) => ws,
            Err(_) => return CfmlValue::Query(CfmlQuery::new(Vec::new())),
        };
        let last_row = ws.highest_row();
        let last_col = ws.highest_column();
        if last_row == 0 || last_col == 0 {
            return CfmlValue::Query(CfmlQuery::new(Vec::new()));
        }
        let mut columns = Vec::with_capacity(last_col as usize);
        for c in 1..=last_col {
            let name = ws.value((c, 1u32));
            columns.push(if name.is_empty() { format!("column_{}", c) } else { name });
        }
        let mut rows: Vec<ValueMap> = Vec::new();
        for r in 2..=last_row {
            let mut m = ValueMap::default();
            for (i, name) in columns.iter().enumerate() {
                m.insert(name.clone(), CfmlValue::string(ws.value(((i as u32) + 1, r))));
            }
            rows.push(m);
        }
        CfmlValue::Query(CfmlQuery::from_parts(columns, rows))
    }

    /// The active sheet as an array of row-arrays (raw grid of cell strings).
    fn to_array(&self) -> CfmlValue {
        let ws = match self.book.sheet(self.active_sheet) {
            Ok(ws) => ws,
            Err(_) => return CfmlValue::array(Vec::new()),
        };
        let last_row = ws.highest_row();
        let last_col = ws.highest_column();
        let mut out = Vec::with_capacity(last_row as usize);
        for r in 1..=last_row {
            let mut row = Vec::with_capacity(last_col as usize);
            for c in 1..=last_col {
                row.push(CfmlValue::string(ws.value((c, r))));
            }
            out.push(CfmlValue::array(row));
        }
        CfmlValue::array(out)
    }

    /// The active sheet as a CSV string.
    fn to_csv(&self, delimiter: &str) -> String {
        let ws = match self.book.sheet(self.active_sheet) {
            Ok(ws) => ws,
            Err(_) => return String::new(),
        };
        let last_row = ws.highest_row();
        let last_col = ws.highest_column();
        let mut out = String::new();
        for r in 1..=last_row {
            let mut fields = Vec::with_capacity(last_col as usize);
            for c in 1..=last_col {
                fields.push(csv_escape(&ws.value((c, r)), delimiter));
            }
            out.push_str(&fields.join(delimiter));
            out.push('\n');
        }
        out
    }

    /// Build a workbook from CSV text (one sheet, "Sheet1").
    fn from_csv_text(text: &str, delimiter: char) -> CfmlSpreadsheet {
        let mut ss = CfmlSpreadsheet::new_xlsx(None);
        let rows = parse_csv(text, delimiter);
        for (ri, row) in rows.iter().enumerate() {
            for (ci, field) in row.iter().enumerate() {
                let _ = ss.set_cell_value(&CfmlValue::string(field.clone()), (ri as u32) + 1, (ci as u32) + 1);
            }
        }
        ss
    }

    /// Read a legacy `.xls`/`.xlsb`/`.ods` (or any calamine-supported) workbook
    /// into a fresh umya `.xlsx` model — data only (styling is not carried over;
    /// calamine reads values, not formats). Each source sheet becomes a sheet.
    fn read_legacy(path: &str) -> Result<CfmlSpreadsheet, CfmlError> {
        use calamine::Reader;
        let mut src = calamine::open_workbook_auto(path)
            .map_err(|e| CfmlError::runtime(format!("Unable to read spreadsheet [{}]: {}", path, e)))?;
        let mut ss = CfmlSpreadsheet::new_xlsx(None);
        let names = src.sheet_names().to_vec();
        for (i, name) in names.iter().enumerate() {
            if i == 0 {
                ss.rename_sheet(name, 1)?;
            } else {
                ss.create_sheet(name)?;
            }
            ss.set_active_sheet_by_name(name)?;
            if let Ok(range) = src.worksheet_range(name) {
                for (r, row) in range.rows().enumerate() {
                    for (c, cell) in row.iter().enumerate() {
                        let v = calamine_cell_to_cfml(cell);
                        if !matches!(v, CfmlValue::Null) {
                            ss.set_cell_value(&v, (r as u32) + 1, (c as u32) + 1)?;
                        }
                    }
                }
            }
        }
        // Reset the active sheet to the first.
        ss.active_sheet = 0;
        Ok(ss)
    }

    // ---- comment / hyperlink getters --------------------------------------

    fn get_cell_comment(&self, row: u32, col: u32) -> CfmlValue {
        if let Ok(ws) = self.book.sheet(self.active_sheet) {
            let target = a1(col, row);
            for c in ws.comments() {
                if c.coordinate().get_coordinate().eq_ignore_ascii_case(&target) {
                    let mut m = ValueMap::default();
                    let text = c.text().text().map(|t| t.value().to_string()).unwrap_or_default();
                    m.insert("comment".to_string(), CfmlValue::string(text));
                    m.insert("author".to_string(), CfmlValue::string(c.author().to_string()));
                    return CfmlValue::strukt(m);
                }
            }
        }
        CfmlValue::strukt(ValueMap::default())
    }

    fn get_cell_hyperlink(&self, row: u32, col: u32) -> CfmlValue {
        if let Ok(ws) = self.book.sheet(self.active_sheet) {
            if let Some(cell) = ws.cell((col, row)) {
                if let Some(hl) = cell.hyperlink() {
                    return CfmlValue::string(hl.url().to_string());
                }
            }
        }
        CfmlValue::string(String::new())
    }

    // ---- split pane / print / header-footer -------------------------------

    fn add_split_pane(&mut self, x_split: f64, y_split: f64, left_col: u32, top_row: u32, active_pane: &str) -> Result<(), CfmlError> {
        let mut pane = Pane::default();
        pane.set_state(PaneStateValues::Split);
        pane.set_horizontal_split(x_split);
        pane.set_vertical_split(y_split);
        let mut tl = Coordinate::default();
        tl.set_col_num(left_col.max(1));
        tl.set_row_num(top_row.max(1));
        pane.set_top_left_cell(tl);
        pane.set_active_pane(match active_pane.to_ascii_uppercase().as_str() {
            "LOWER_LEFT" => PaneValues::BottomLeft,
            "LOWER_RIGHT" => PaneValues::BottomRight,
            "UPPER_RIGHT" => PaneValues::TopRight,
            _ => PaneValues::TopLeft,
        });
        let views = self.active_ws_mut()?.sheet_views_mut();
        if views.sheet_view_list_mut().is_empty() {
            views.add_sheet_view_list_mut(SheetView::default());
        }
        views.sheet_view_list_mut()[0].set_pane(pane);
        Ok(())
    }

    fn set_print_orientation(&mut self, mode: &str) -> Result<(), CfmlError> {
        let o = match mode.to_ascii_lowercase().as_str() {
            "landscape" => OrientationValues::Landscape,
            "portrait" => OrientationValues::Portrait,
            _ => OrientationValues::Default,
        };
        self.active_ws_mut()?.page_setup_mut().set_orientation(o);
        Ok(())
    }

    fn set_fit_to_page(&mut self, state: bool, pages_wide: u32, pages_high: u32) -> Result<(), CfmlError> {
        let ps = self.active_ws_mut()?.page_setup_mut();
        if state {
            ps.set_fit_to_width(pages_wide.max(1));
            ps.set_fit_to_height(pages_high.max(1));
        } else {
            ps.set_fit_to_width(0);
            ps.set_fit_to_height(0);
        }
        Ok(())
    }

    fn set_header(&mut self, left: &str, center: &str, right: &str) -> Result<(), CfmlError> {
        let mut h = OddHeader::default();
        h.set_value(format!("&L{}&C{}&R{}", left, center, right));
        self.active_ws_mut()?.header_footer_mut().set_odd_header(h);
        Ok(())
    }

    fn set_footer(&mut self, left: &str, center: &str, right: &str) -> Result<(), CfmlError> {
        let mut f = OddFooter::default();
        f.set_value(format!("&L{}&C{}&R{}", left, center, right));
        self.active_ws_mut()?.header_footer_mut().set_odd_footer(f);
        Ok(())
    }

    // ---- data validation / conditional formatting -------------------------

    /// Add a data-validation rule over `range`. `dv_type` = list/whole/decimal/
    /// textLength/date/custom; `formula1`/`formula2` are the bound expressions
    /// (for a list, `formula1` is e.g. `"\"A,B,C\""` or a range reference).
    fn add_data_validation(&mut self, range: &str, dv_type: &str, operator: &str, formula1: &str, formula2: &str) -> Result<(), CfmlError> {
        let mut dv = DataValidation::default();
        dv.set_type(match dv_type.to_ascii_lowercase().as_str() {
            "whole" => DataValidationValues::Whole,
            "decimal" => DataValidationValues::Decimal,
            "textlength" => DataValidationValues::TextLength,
            "date" => DataValidationValues::Date,
            "custom" => DataValidationValues::Custom,
            _ => DataValidationValues::List,
        });
        if !operator.is_empty() {
            dv.set_operator(match operator.to_ascii_lowercase().as_str() {
                "greaterthan" => DataValidationOperatorValues::GreaterThan,
                "greaterthanorequal" => DataValidationOperatorValues::GreaterThanOrEqual,
                "lessthan" => DataValidationOperatorValues::LessThan,
                "lessthanorequal" => DataValidationOperatorValues::LessThanOrEqual,
                "notbetween" => DataValidationOperatorValues::NotBetween,
                "equal" => DataValidationOperatorValues::Equal,
                "notequal" => DataValidationOperatorValues::NotEqual,
                _ => DataValidationOperatorValues::Between,
            });
        }
        if !formula1.is_empty() {
            dv.set_formula1(formula1.to_string());
        }
        if !formula2.is_empty() {
            dv.set_formula2(formula2.to_string());
        }
        dv.set_allow_blank(true);
        dv.sequence_of_references_mut().set_sqref(range.to_string());
        let ws = self.active_ws_mut()?;
        match ws.data_validations_mut() {
            Some(list) => { list.add_data_validation_list(dv); }
            None => {
                let mut list = DataValidations::default();
                list.add_data_validation_list(dv);
                ws.set_data_validations(list);
            }
        }
        Ok(())
    }

    /// Add a `cellIs` conditional-formatting rule (`operator` on `value`) over
    /// `range`, applying `fmt` styling when the condition matches.
    fn add_conditional_formatting(&mut self, range: &str, operator: &str, value: &str, fmt: &CfmlStruct) -> Result<(), CfmlError> {
        let mut rule = ConditionalFormattingRule::default();
        rule.set_type(ConditionalFormatValues::CellIs);
        rule.set_operator(match operator.to_ascii_lowercase().as_str() {
            "greaterthan" => ConditionalFormattingOperatorValues::GreaterThan,
            "greaterthanorequal" => ConditionalFormattingOperatorValues::GreaterThanOrEqual,
            "lessthan" => ConditionalFormattingOperatorValues::LessThan,
            "lessthanorequal" => ConditionalFormattingOperatorValues::LessThanOrEqual,
            "notequal" => ConditionalFormattingOperatorValues::NotEqual,
            "between" => ConditionalFormattingOperatorValues::Between,
            _ => ConditionalFormattingOperatorValues::Equal,
        });
        rule.set_priority(1);
        let mut f = Formula::default();
        f.set_string_value(value.to_string());
        rule.set_formula(f);
        let mut style = Style::default();
        apply_format(&mut style, fmt);
        rule.set_style(style);

        let mut cf = ConditionalFormatting::default();
        cf.set_sequence_of_references({
            let mut s = umya_spreadsheet::SequenceOfReferences::default();
            s.set_sqref(range.to_string());
            s
        });
        cf.add_conditional_collection(rule);
        self.active_ws_mut()?.add_conditional_formatting_collection(cf);
        Ok(())
    }

    /// Add a 2- or 3-colour scale conditional format across `range`.
    fn add_color_scale(&mut self, range: &str, colors: &[String]) -> Result<(), CfmlError> {
        let mut scale = ColorScale::default();
        // Anchor points: min / (mid) / max.
        let points: &[(ConditionalFormatValueObjectValues, &str)] = if colors.len() >= 3 {
            &[
                (ConditionalFormatValueObjectValues::Min, ""),
                (ConditionalFormatValueObjectValues::Percentile, "50"),
                (ConditionalFormatValueObjectValues::Max, ""),
            ]
        } else {
            &[
                (ConditionalFormatValueObjectValues::Min, ""),
                (ConditionalFormatValueObjectValues::Max, ""),
            ]
        };
        for (ty, val) in points {
            let mut cfvo = ConditionalFormatValueObject::default();
            cfvo.set_type(ty.clone());
            if !val.is_empty() {
                cfvo.set_val(val.to_string());
            }
            scale.add_cfvo_collection(cfvo);
        }
        for c in colors {
            let mut color = Color::default();
            color.set_argb_str(resolve_color(c));
            scale.add_color_collection(color);
        }
        let mut rule = ConditionalFormattingRule::default();
        rule.set_type(ConditionalFormatValues::ColorScale);
        rule.set_priority(1);
        rule.set_color_scale(scale);
        let mut cf = ConditionalFormatting::default();
        cf.set_sequence_of_references({
            let mut s = umya_spreadsheet::SequenceOfReferences::default();
            s.set_sqref(range.to_string());
            s
        });
        cf.add_conditional_collection(rule);
        self.active_ws_mut()?.add_conditional_formatting_collection(cf);
        Ok(())
    }

    fn get_column_width(&self, col: u32) -> f64 {
        self.book
            .sheet(self.active_sheet)
            .ok()
            .and_then(|ws| ws.column_dimension(&col_letter(col)).map(|c| c.width()))
            .unwrap_or(0.0)
    }

    fn get_cell_format(&self, row: u32, col: u32) -> CfmlValue {
        let mut m = ValueMap::default();
        if let Ok(ws) = self.book.sheet(self.active_sheet) {
            if let Some(cell) = ws.cell((col, row)) {
                let style = cell.style();
                if let Some(f) = style.font() {
                    m.insert("bold".to_string(), CfmlValue::Bool(f.bold()));
                    m.insert("italic".to_string(), CfmlValue::Bool(f.italic()));
                    m.insert("fontsize".to_string(), CfmlValue::Double(f.size()));
                    m.insert("font".to_string(), CfmlValue::string(f.name().to_string()));
                    m.insert("color".to_string(), CfmlValue::string(hexify(f.color().argb_str())));
                }
                if let Some(bg) = style.background_color() {
                    m.insert("bgcolor".to_string(), CfmlValue::string(hexify(bg.argb_str())));
                }
                if let Some(al) = style.alignment() {
                    m.insert("alignment".to_string(), CfmlValue::string(halign_str(al.horizontal())));
                    m.insert("verticalalignment".to_string(), CfmlValue::string(valign_str(al.vertical())));
                    m.insert("wraptext".to_string(), CfmlValue::Bool(al.wrap_text()));
                }
                if let Some(nf) = style.numbering_format() {
                    m.insert("dataformat".to_string(), CfmlValue::string(nf.format_code().to_string()));
                }
            }
        }
        CfmlValue::strukt(m)
    }

    fn set_active_cell(&mut self, row: u32, col: u32) -> Result<(), CfmlError> {
        self.active_ws_mut()?.set_active_cell(a1(col, row));
        Ok(())
    }

    fn add_page_breaks(&mut self, row_breaks: &[u32], col_breaks: &[u32]) -> Result<(), CfmlError> {
        let ws = self.active_ws_mut()?;
        for r in row_breaks {
            let mut b = Break::default();
            b.set_id(*r);
            b.set_manual_page_break(true);
            ws.row_breaks_mut().add_break_list(b);
        }
        for c in col_breaks {
            let mut b = Break::default();
            b.set_id(*c);
            b.set_manual_page_break(true);
            ws.column_breaks_mut().add_break_list(b);
        }
        Ok(())
    }

    /// Set print-title rows/columns via the `_xlnm.Print_Titles` defined name.
    fn set_repeating(&mut self, address: &str) -> Result<(), CfmlError> {
        let sheet_name = self.active_ws_mut()?.name().to_string();
        let full = format!("'{}'!{}", sheet_name, address);
        self.active_ws_mut()?
            .add_defined_name("_xlnm.Print_Titles", &full)
            .map_err(|e| CfmlError::runtime(format!("setRepeating: {}", e)))
    }

    fn add_image_bytes(&mut self, bytes: &[u8], name: &str, anchor: &str) -> Result<(), CfmlError> {
        let dims = imagesize::blob_size(bytes)
            .map_err(|e| CfmlError::runtime(format!("addImage: cannot read image dimensions: {}", e)))?;
        let mut marker = MarkerType::default();
        marker.set_coordinate(anchor_to_a1(anchor));
        let mut img = XlsxImage::default();
        img.new_image_with_dimensions(dims.height as u32, dims.width as u32, name, bytes.to_vec(), marker);
        self.active_ws_mut()?.add_image(img);
        Ok(())
    }

    fn load(&mut self, path: &str) -> Result<(), CfmlError> {
        let book = umya_spreadsheet::reader::xlsx::read(std::path::Path::new(path))
            .map_err(|e| CfmlError::runtime(format!("Unable to load spreadsheet [{}]: {:?}", path, e)))?;
        self.book = book;
        self.active_sheet = 0;
        self.format = WorkbookFormat::Xlsx;
        Ok(())
    }

    /// Active sheet → JSON array of row objects (row 1 = keys).
    fn to_json(&self, pretty: bool) -> String {
        use serde_json::{Map, Value};
        let ws = match self.book.sheet(self.active_sheet) {
            Ok(ws) => ws,
            Err(_) => return "[]".to_string(),
        };
        let last_row = ws.highest_row();
        let last_col = ws.highest_column();
        if last_row == 0 || last_col == 0 {
            return "[]".to_string();
        }
        let headers: Vec<String> = (1..=last_col)
            .map(|c| {
                let n = ws.value((c, 1u32));
                if n.is_empty() { format!("column_{}", c) } else { n }
            })
            .collect();
        let mut arr = Vec::new();
        for r in 2..=last_row {
            let mut obj = Map::new();
            for (i, h) in headers.iter().enumerate() {
                obj.insert(h.clone(), Value::String(ws.value(((i as u32) + 1, r))));
            }
            arr.push(Value::Object(obj));
        }
        let v = Value::Array(arr);
        if pretty {
            serde_json::to_string_pretty(&v).unwrap_or_else(|_| "[]".to_string())
        } else {
            serde_json::to_string(&v).unwrap_or_else(|_| "[]".to_string())
        }
    }

    /// Build a workbook from a JSON array (of objects → header + rows, or of
    /// arrays → raw grid).
    fn from_json_text(text: &str) -> Result<CfmlSpreadsheet, CfmlError> {
        use serde_json::Value;
        let v: Value = serde_json::from_str(text)
            .map_err(|e| CfmlError::runtime(format!("fromJson: invalid JSON: {}", e)))?;
        let mut ss = CfmlSpreadsheet::new_xlsx(None);
        if let Value::Array(rows) = v {
            if let Some(Value::Object(first)) = rows.iter().find(|r| matches!(r, Value::Object(_))) {
                let headers: Vec<String> = first.keys().cloned().collect();
                for (i, h) in headers.iter().enumerate() {
                    ss.set_cell_value(&CfmlValue::string(h.clone()), 1, (i as u32) + 1)?;
                }
                for (ri, row) in rows.iter().enumerate() {
                    if let Value::Object(o) = row {
                        for (i, h) in headers.iter().enumerate() {
                            let cell = o.get(h).map(json_to_cfml).unwrap_or(CfmlValue::Null);
                            ss.set_cell_value(&cell, (ri as u32) + 2, (i as u32) + 1)?;
                        }
                    }
                }
            } else {
                for (ri, row) in rows.iter().enumerate() {
                    if let Value::Array(cells) = row {
                        for (ci, c) in cells.iter().enumerate() {
                            ss.set_cell_value(&json_to_cfml(c), (ri as u32) + 1, (ci as u32) + 1)?;
                        }
                    }
                }
            }
        }
        Ok(ss)
    }
}

/// Map horizontal-alignment enum → CFML string.
fn halign_str(v: &HorizontalAlignmentValues) -> String {
    match v {
        HorizontalAlignmentValues::Left => "left",
        HorizontalAlignmentValues::Center => "center",
        HorizontalAlignmentValues::Right => "right",
        HorizontalAlignmentValues::Justify => "justify",
        _ => "general",
    }
    .to_string()
}

fn valign_str(v: &VerticalAlignmentValues) -> String {
    match v {
        VerticalAlignmentValues::Top => "top",
        VerticalAlignmentValues::Center => "center",
        VerticalAlignmentValues::Bottom => "bottom",
        _ => "bottom",
    }
    .to_string()
}

/// Normalise umya's `AARRGGBB` string to a CFML `#RRGGBB` (drops opaque alpha).
fn hexify(argb: String) -> String {
    if argb.len() == 8 && argb.to_uppercase().starts_with("FF") {
        format!("#{}", &argb[2..])
    } else if argb.len() == 8 {
        format!("#{}", argb)
    } else {
        argb
    }
}

/// Map a serde_json value to a CfmlValue.
fn json_to_cfml(v: &serde_json::Value) -> CfmlValue {
    use serde_json::Value;
    match v {
        Value::String(s) => CfmlValue::string(s.clone()),
        Value::Bool(b) => CfmlValue::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                CfmlValue::Int(i)
            } else {
                CfmlValue::Double(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::Null => CfmlValue::Null,
        other => CfmlValue::string(other.to_string()),
    }
}

/// Map a calamine cell value to a CfmlValue.
fn calamine_cell_to_cfml(cell: &calamine::Data) -> CfmlValue {
    use calamine::Data;
    match cell {
        Data::Int(i) => CfmlValue::Int(*i),
        Data::Float(f) => CfmlValue::Double(*f),
        Data::String(s) => CfmlValue::string(s.clone()),
        Data::Bool(b) => CfmlValue::Bool(*b),
        Data::DateTime(dt) => CfmlValue::string(dt.to_string()),
        Data::DateTimeIso(s) | Data::DurationIso(s) => CfmlValue::string(s.clone()),
        Data::Error(_) => CfmlValue::string(String::new()),
        Data::Empty => CfmlValue::Null,
    }
}

/// Parse a range spec like `"2-5,7,9-10"` into a flat list of 1-based indices.
fn parse_range_list(spec: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for part in spec.split([',', ';']) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((a, b)) = part.split_once('-') {
            if let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                for n in a.min(b)..=a.max(b) {
                    out.push(n);
                }
            }
        } else if let Ok(n) = part.parse::<u32>() {
            out.push(n);
        }
    }
    out
}

/// Turn a CFML anchor (`"A1"`, or `"row,col"` / `"startRow,startCol,..."`) into
/// an A1 top-left reference for image/marker placement.
fn anchor_to_a1(anchor: &str) -> String {
    let a = anchor.trim();
    if a.is_empty() {
        return "A1".to_string();
    }
    // "row,col[,...]" numeric form → A1.
    if a.contains(',') {
        let parts: Vec<&str> = a.split(',').collect();
        if let (Some(r), Some(c)) = (parts.first(), parts.get(1)) {
            if let (Ok(r), Ok(c)) = (r.trim().parse::<u32>(), c.trim().parse::<u32>()) {
                return a1(c.max(1), r.max(1));
            }
        }
    }
    a.to_string() // assume already an A1 reference
}

/// Escape a CSV field: quote when it contains the delimiter, a quote, or a newline.
fn csv_escape(field: &str, delimiter: &str) -> String {
    let needs_quote = field.contains(delimiter) || field.contains('"') || field.contains('\n') || field.contains('\r');
    if needs_quote {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Minimal RFC-4180-ish CSV parser (handles quoted fields, escaped quotes,
/// embedded delimiters/newlines).
fn parse_csv(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                field.push(ch);
            }
        } else if ch == '"' {
            in_quotes = true;
        } else if ch == delimiter {
            row.push(std::mem::take(&mut field));
        } else if ch == '\n' {
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
        } else if ch == '\r' {
            // swallow; the following \n ends the row
        } else {
            field.push(ch);
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Convert a CFML value into a row/column of cell values: an array yields its
/// elements; anything else is treated as a delimited list.
fn value_to_cells(v: &CfmlValue, delimiter: &str) -> Vec<CfmlValue> {
    match v {
        CfmlValue::Array(a) => a.snapshot(),
        CfmlValue::QueryColumn(a, _) => (**a).clone(),
        CfmlValue::Null => Vec::new(),
        other => {
            let s = other.as_string();
            if s.is_empty() {
                Vec::new()
            } else {
                s.split(delimiter).map(|p| CfmlValue::string(p.to_string())).collect()
            }
        }
    }
}

/// 1-based column index → spreadsheet column letters ("A", "Z", "AA", …).
fn col_letter(mut col: u32) -> String {
    let mut s = String::new();
    while col > 0 {
        let rem = ((col - 1) % 26) as u8;
        s.insert(0, (b'A' + rem) as char);
        col = (col - 1) / 26;
    }
    if s.is_empty() { s.push('A'); }
    s
}

/// 1-based (col,row) → A1 reference.
fn a1(col: u32, row: u32) -> String {
    format!("{}{}", col_letter(col), row)
}

/// Resolve a CFML colour (named or `#RGB`/`#RRGGBB`/`AARRGGBB` hex) to umya's
/// `AARRGGBB` ARGB hex string.
fn resolve_color(input: &str) -> String {
    let s = input.trim();
    let s = s.strip_prefix('#').unwrap_or(s);
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return match s.len() {
            8 => s.to_uppercase(),
            6 => format!("FF{}", s.to_uppercase()),
            3 => {
                let mut o = String::from("FF");
                for c in s.chars() {
                    o.push(c);
                    o.push(c);
                }
                o.to_uppercase()
            }
            _ => "FF000000".to_string(),
        };
    }
    let hex = match s.to_ascii_lowercase().as_str() {
        "black" => "000000",
        "white" => "FFFFFF",
        "red" => "FF0000",
        "green" => "008000",
        "lime" => "00FF00",
        "blue" => "0000FF",
        "yellow" => "FFFF00",
        "cyan" | "aqua" => "00FFFF",
        "magenta" | "fuchsia" => "FF00FF",
        "orange" => "FFA500",
        "pink" => "FFC0CB",
        "purple" => "800080",
        "gray" | "grey" => "808080",
        "darkgray" | "darkgrey" => "A9A9A9",
        "lightgray" | "lightgrey" => "D3D3D3",
        "darkblue" => "000080",
        "lightblue" => "ADD8E6",
        "darkgreen" => "006400",
        "darkred" => "8B0000",
        "brown" => "A52A2A",
        _ => "000000",
    };
    format!("FF{}", hex)
}

/// Apply a CFML format struct (`{bold,italic,underline,font,fontsize,color,
/// bgcolor,alignment,verticalalignment,dataformat,wraptext,…}`) to a cell style.
/// Keys are read case-insensitively; unknown keys are ignored.
fn apply_format(style: &mut Style, fmt: &CfmlStruct) {
    let get = |keys: &[&str]| -> Option<CfmlValue> {
        keys.iter().find_map(|k| fmt.get_ci(k))
    };
    if let Some(v) = get(&["bold"]) {
        style.font_mut().set_bold(v.is_true());
    }
    if let Some(v) = get(&["italic"]) {
        style.font_mut().set_italic(v.is_true());
    }
    if let Some(v) = get(&["underline"]) {
        if v.is_true() {
            style.font_mut().set_underline("single");
        }
    }
    if let Some(v) = get(&["fontsize", "size"]) {
        if let Ok(n) = v.as_string().trim().parse::<f64>() {
            style.font_mut().set_size(n);
        }
    }
    if let Some(v) = get(&["font", "fontname"]) {
        style.font_mut().set_name(v.as_string());
    }
    if let Some(v) = get(&["color", "fontcolor"]) {
        style.font_mut().color_mut().set_argb_str(resolve_color(&v.as_string()));
    }
    if let Some(v) = get(&["bgcolor", "fgcolor", "backgroundcolor"]) {
        style.set_background_color_solid(resolve_color(&v.as_string()));
    }
    if let Some(v) = get(&["dataformat", "numberformat"]) {
        let mut nf = NumberingFormat::default();
        nf.set_format_code(v.as_string());
        style.set_numbering_format(nf);
    }
    if let Some(v) = get(&["alignment", "horizontalalignment"]) {
        let al = style.alignment_mut();
        match v.as_string().to_ascii_lowercase().as_str() {
            "left" => al.set_horizontal(HorizontalAlignmentValues::Left),
            "center" | "centre" => al.set_horizontal(HorizontalAlignmentValues::Center),
            "right" => al.set_horizontal(HorizontalAlignmentValues::Right),
            "justify" => al.set_horizontal(HorizontalAlignmentValues::Justify),
            _ => {}
        }
    }
    if let Some(v) = get(&["verticalalignment"]) {
        let al = style.alignment_mut();
        match v.as_string().to_ascii_lowercase().as_str() {
            "top" => al.set_vertical(VerticalAlignmentValues::Top),
            "center" | "centre" | "middle" => al.set_vertical(VerticalAlignmentValues::Center),
            "bottom" => al.set_vertical(VerticalAlignmentValues::Bottom),
            _ => {}
        }
    }
    if let Some(v) = get(&["wraptext"]) {
        style.alignment_mut().set_wrap_text(v.is_true());
    }
}

/// Split format-method args into the (single) format struct and the ordered
/// numeric coordinates. This lets BOTH the ACF/cfsimplicity ordering
/// (`format` first) and the BoxLang fluent ordering (coords first) work, since
/// the format is always a struct and coordinates are always numeric.
fn split_format_args(a: &[CfmlValue]) -> (CfmlStruct, Vec<u32>) {
    let mut fmt = CfmlStruct::empty();
    let mut nums = Vec::new();
    for v in a {
        match v {
            CfmlValue::Struct(s) => fmt = s.clone(),
            other => {
                if let Ok(n) = other.as_string().trim().parse::<f64>() {
                    if n >= 1.0 {
                        nums.push(n as u32);
                    }
                }
            }
        }
    }
    (fmt, nums)
}

/// Read a positional numeric arg (1-based row/col) from the slice.
fn arg_u32(args: &[CfmlValue], idx: usize, what: &str) -> Result<u32, CfmlError> {
    match args.get(idx) {
        Some(v) => {
            let n = v.as_string().trim().parse::<f64>().map_err(|_| {
                CfmlError::runtime(format!("Spreadsheet: {} must be numeric", what))
            })?;
            if n < 1.0 {
                return Err(CfmlError::runtime(format!("Spreadsheet: {} is 1-based", what)));
            }
            Ok(n as u32)
        }
        None => Err(CfmlError::runtime(format!("Spreadsheet: missing {}", what))),
    }
}

impl CfmlNative for CfmlSpreadsheet {
    fn class_name(&self) -> &str {
        "Spreadsheet"
    }

    /// Declared parameter names, so `wb.formatRow( row=1, format={bold:true} )`
    /// binds by name instead of by call-site order. Names follow the
    /// ACF/Lucee/cfsimplicity `Spreadsheet*` BIF signatures (minus the leading
    /// workbook argument, which the member form supplies as the receiver).
    fn method_params(&self, method: &str) -> Option<&'static [&'static str]> {
        Some(match method.to_lowercase().as_str() {
            "setcellvalue" => &["value", "row", "column", "type"][..],
            "createsheet" | "newsheet" | "setactivesheet" | "sheet" => &["sheetName"][..],
            "renamesheet" => &["sheetName", "sheetNumber"][..],
            "write" => &["filepath", "overwrite", "password"][..],
            "setactivesheetnumber" => &["sheetNumber"][..],
            "addrow" => &["data", "row", "column", "insert", "delimiter"][..],
            "addrows" => &["data", "row", "column", "includeQueryColumnNames"][..],
            "addcolumn" => &["data", "startRow", "startColumn", "insert", "delimiter"][..],
            "formatcell" => &["format", "row", "column"][..],
            "formatrow" => &["format", "row"][..],
            "formatcolumn" => &["format", "column"][..],
            "formatcellrange" => {
                &["format", "startRow", "startColumn", "endRow", "endColumn"][..]
            }
            "mergecells" | "clearcellrange" => {
                &["startRow", "startColumn", "endRow", "endColumn"][..]
            }
            "addfreezepane" => &["freezeColumn", "freezeRow"][..],
            "freezerows" | "setrepeatingrows" | "deleterows" => &["rows"][..],
            "freezecols" | "freezecolumns" | "setrepeatingcolumns" | "deletecolumns" => {
                &["columns"][..]
            }
            "autosizecolumn" | "deletecolumn" | "hidecolumn" | "showcolumn"
            | "iscolumnhidden" | "getcolumnwidth" => &["column"][..],
            "setcolumnwidth" => &["column", "width"][..],
            "setrowheight" => &["row", "height"][..],
            "deleterow" | "hiderow" | "showrow" | "isrowhidden" => &["row"][..],
            "shiftrows" | "shiftcolumns" => &["start", "end", "offset"][..],
            "setcellformula" => &["formula", "row", "column"][..],
            "clearcell" | "setactivecell" | "getcellcomment" | "getcellhyperlink"
            | "getcellformula" | "getcelltype" | "getcellformat" | "getcellvalue" => {
                &["row", "column"][..]
            }
            "setcellrangevalue" => {
                &["value", "startRow", "startColumn", "endRow", "endColumn"][..]
            }
            "setcolumnhidden" => &["column", "hidden"][..],
            "setrowhidden" => &["row", "hidden"][..],
            "setcellcomment" => &["comment", "row", "column"][..],
            "setcellhyperlink" => &["link", "row", "column", "tooltip"][..],
            "addautofilter" => &["range"][..],
            "addinfo" => &["info"][..],
            "addimage" => &["image", "anchor"][..],
            "addchart" => &["chart"][..],
            "fromquery" | "fromarray" => &["data", "includeHeaders"][..],
            "fromcsv" => &["csv", "delimiter"][..],
            "selectsheet" => &["sheet"][..],
            "headerrow" => &["data"][..],
            "writetocsv" | "writecsv" => &["filepath", "delimiter"][..],
            "addsplitpane" => {
                &[
                    "xSplitPosition",
                    "ySplitPosition",
                    "leftmostColumn",
                    "topRow",
                    "activePane",
                ][..]
            }
            "setprintorientation" => &["orientation"][..],
            "setfittopage" => &["state", "pagesWide", "pagesHigh"][..],
            "setheader" | "setfooter" => &["left", "center", "right"][..],
            "adddatavalidation" => &["validation"][..],
            "addconditionalformatting" | "addconditionalformat" => &["format"][..],
            "addpagebreaks" => &["rowBreaks", "columnBreaks"][..],
            "load" => &["filepath"][..],
            "fromjson" => &["json"][..],
            "tojson" => &["pretty"][..],
            "tocsv" => &["delimiter"][..],
            // No-argument methods: naming anything is an error, which is what an
            // empty parameter list produces.
            "autosize" | "readbinary" | "tobinary" | "toquery" | "toarray"
            | "getrowcount" | "rowcount" | "getcolumncount" | "columncount" | "info" => &[][..],
            _ => return None,
        })
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        let a = args.as_slice();
        match name.to_lowercase().as_str() {
            // ---- mutators: return the fluent self-handle -------------------
            "setcellvalue" => {
                // (value, row, column[, type])
                let value = a.first().cloned().unwrap_or(CfmlValue::Null);
                let row = arg_u32(a, 1, "row")?;
                let col = arg_u32(a, 2, "column")?;
                self.set_cell_value(&value, row, col)?;
                Ok(self.this())
            }
            "createsheet" | "newsheet" => {
                let sheet_name = a.first().map(|v| v.as_string()).unwrap_or_default();
                self.create_sheet(&sheet_name)?;
                Ok(self.this())
            }
            "renamesheet" => {
                // (sheetName, sheetNumber)
                let sheet_name = a.first().map(|v| v.as_string()).unwrap_or_default();
                let num = a.get(1).map(|v| v.as_string().trim().parse::<usize>().unwrap_or(1)).unwrap_or(1);
                self.rename_sheet(&sheet_name, num)?;
                Ok(self.this())
            }
            "write" => {
                // (filepath[, overwrite][, password])
                let path = a.first().map(|v| v.as_string()).unwrap_or_default();
                let password = a.get(2).map(|v| v.as_string()).filter(|s| !s.is_empty());
                self.write_to(&path, password.as_deref())?;
                Ok(self.this())
            }
            "setactivesheet" => {
                let name = a.first().map(|v| v.as_string()).unwrap_or_default();
                self.set_active_sheet_by_name(&name)?;
                Ok(self.this())
            }
            "setactivesheetnumber" => {
                let num = a.first().map(|v| v.as_string().trim().parse::<usize>().unwrap_or(1)).unwrap_or(1);
                self.set_active_sheet_number(num)?;
                Ok(self.this())
            }
            "addrow" => {
                // (data[, row][, column=1][, insert=true][, delimiter=","])
                let data = a.first().cloned().unwrap_or(CfmlValue::Null);
                let row = a.get(1).and_then(|v| v.as_string().trim().parse::<u32>().ok())
                    .unwrap_or_else(|| (self.row_count() as u32) + 1);
                let col = a.get(2).and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(1);
                let insert = a.get(3).map(|v| v.is_true()).unwrap_or(true);
                let delim = a.get(4).map(|v| v.as_string()).unwrap_or_else(|| ",".to_string());
                self.add_row(&data, row, col, insert, &delim)?;
                Ok(self.this())
            }
            "addrows" => {
                // (data[, row][, column=1][, includeQueryColumnNames=false])
                let data = a.first().cloned().unwrap_or(CfmlValue::Null);
                let start_row = a.get(1).and_then(|v| v.as_string().trim().parse::<u32>().ok());
                let col = a.get(2).and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(1);
                let headers = a.get(3).map(|v| v.is_true()).unwrap_or(false);
                self.add_rows(&data, start_row, col, headers)?;
                Ok(self.this())
            }
            "addcolumn" => {
                // (data[, startRow=1][, startColumn][, insert=false][, delimiter=","])
                let data = a.first().cloned().unwrap_or(CfmlValue::Null);
                let start_row = a.get(1).and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(1);
                let start_col = a.get(2).and_then(|v| v.as_string().trim().parse::<u32>().ok())
                    .unwrap_or_else(|| (self.column_count() as u32) + 1);
                let insert = a.get(3).map(|v| v.is_true()).unwrap_or(false);
                let delim = a.get(4).map(|v| v.as_string()).unwrap_or_else(|| ",".to_string());
                self.add_column(&data, start_row, start_col, insert, &delim)?;
                Ok(self.this())
            }
            "formatcell" => {
                let (fmt, nums) = split_format_args(a);
                let row = *nums.first().ok_or_else(|| CfmlError::runtime("formatCell: missing row".to_string()))?;
                let col = *nums.get(1).ok_or_else(|| CfmlError::runtime("formatCell: missing column".to_string()))?;
                self.format_cell(&fmt, row, col)?;
                Ok(self.this())
            }
            "formatrow" => {
                let (fmt, nums) = split_format_args(a);
                let row = *nums.first().ok_or_else(|| CfmlError::runtime("formatRow: missing row".to_string()))?;
                self.format_row(&fmt, row)?;
                Ok(self.this())
            }
            "formatcolumn" => {
                let (fmt, nums) = split_format_args(a);
                let col = *nums.first().ok_or_else(|| CfmlError::runtime("formatColumn: missing column".to_string()))?;
                self.format_column(&fmt, col)?;
                Ok(self.this())
            }
            "formatcellrange" => {
                // (format, startRow, startColumn, endRow, endColumn) in numeric order
                let (fmt, n) = split_format_args(a);
                if n.len() < 4 {
                    return Err(CfmlError::runtime("formatCellRange: need startRow,startColumn,endRow,endColumn".to_string()));
                }
                self.format_cell_range(&fmt, n[0], n[1], n[2], n[3])?;
                Ok(self.this())
            }
            "mergecells" => {
                // (startRow, startColumn, endRow, endColumn)
                let sr = arg_u32(a, 0, "startRow")?;
                let sc = arg_u32(a, 1, "startColumn")?;
                let er = arg_u32(a, 2, "endRow")?;
                let ec = arg_u32(a, 3, "endColumn")?;
                self.merge_cells(sr, sc, er, ec)?;
                Ok(self.this())
            }
            "addfreezepane" => {
                // (freezeColumn, freezeRow)
                let fc = a.first().and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(0);
                let fr = a.get(1).and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(0);
                self.add_freeze_pane(fc, fr)?;
                Ok(self.this())
            }
            "freezerows" => {
                let n = a.first().and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(0);
                self.add_freeze_pane(0, n)?;
                Ok(self.this())
            }
            "freezecols" | "freezecolumns" => {
                let n = a.first().and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(0);
                self.add_freeze_pane(n, 0)?;
                Ok(self.this())
            }
            "autosizecolumn" => {
                let col = arg_u32(a, 0, "column")?;
                self.auto_size_column(col)?;
                Ok(self.this())
            }
            "autosize" => {
                // all columns 1..=highest
                let last = (self.column_count() as u32).max(1);
                for col in 1..=last {
                    self.auto_size_column(col)?;
                }
                Ok(self.this())
            }
            "setcolumnwidth" => {
                let col = arg_u32(a, 0, "column")?;
                let w = a.get(1).and_then(|v| v.as_string().trim().parse::<f64>().ok()).unwrap_or(10.0);
                self.set_column_width(col, w)?;
                Ok(self.this())
            }
            "setrowheight" => {
                let row = arg_u32(a, 0, "row")?;
                let h = a.get(1).and_then(|v| v.as_string().trim().parse::<f64>().ok()).unwrap_or(15.0);
                self.set_row_height(row, h)?;
                Ok(self.this())
            }
            "deleterow" => { let r = arg_u32(a, 0, "row")?; self.delete_row(r)?; Ok(self.this()) }
            "deleterows" => { self.delete_rows(&a.first().map(|v| v.as_string()).unwrap_or_default())?; Ok(self.this()) }
            "deletecolumn" => { let c = arg_u32(a, 0, "column")?; self.delete_column(c)?; Ok(self.this()) }
            "deletecolumns" => { self.delete_columns(&a.first().map(|v| v.as_string()).unwrap_or_default())?; Ok(self.this()) }
            "shiftrows" => {
                let start = arg_u32(a, 0, "start")?;
                let end = a.get(1).and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(start);
                let offset = a.get(2).and_then(|v| v.as_string().trim().parse::<i32>().ok()).unwrap_or(1);
                self.shift_rows(start, end, offset)?;
                Ok(self.this())
            }
            "shiftcolumns" => {
                let start = arg_u32(a, 0, "start")?;
                let end = a.get(1).and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(start);
                let offset = a.get(2).and_then(|v| v.as_string().trim().parse::<i32>().ok()).unwrap_or(1);
                self.shift_columns(start, end, offset)?;
                Ok(self.this())
            }
            "setcellformula" => {
                let formula = a.first().map(|v| v.as_string()).unwrap_or_default();
                let row = arg_u32(a, 1, "row")?;
                let col = arg_u32(a, 2, "column")?;
                self.set_cell_formula(&formula, row, col)?;
                Ok(self.this())
            }
            "clearcell" => {
                let row = arg_u32(a, 0, "row")?;
                let col = arg_u32(a, 1, "column")?;
                self.clear_cell(row, col)?;
                Ok(self.this())
            }
            "clearcellrange" => {
                let sr = arg_u32(a, 0, "startRow")?;
                let sc = arg_u32(a, 1, "startColumn")?;
                let er = arg_u32(a, 2, "endRow")?;
                let ec = arg_u32(a, 3, "endColumn")?;
                self.clear_cell_range(sr, sc, er, ec)?;
                Ok(self.this())
            }
            "setcellrangevalue" => {
                let value = a.first().cloned().unwrap_or(CfmlValue::Null);
                let sr = arg_u32(a, 1, "startRow")?;
                let sc = arg_u32(a, 2, "startColumn")?;
                let er = arg_u32(a, 3, "endRow")?;
                let ec = arg_u32(a, 4, "endColumn")?;
                self.set_cell_range_value(&value, sr, sc, er, ec)?;
                Ok(self.this())
            }
            "setcolumnhidden" => {
                let col = arg_u32(a, 0, "column")?;
                let hidden = a.get(1).map(|v| v.is_true()).unwrap_or(true);
                self.set_column_hidden(col, hidden)?;
                Ok(self.this())
            }
            "setrowhidden" => {
                let row = arg_u32(a, 0, "row")?;
                let hidden = a.get(1).map(|v| v.is_true()).unwrap_or(true);
                self.set_row_hidden(row, hidden)?;
                Ok(self.this())
            }
            "hidecolumn" => { let c = arg_u32(a, 0, "column")?; self.set_column_hidden(c, true)?; Ok(self.this()) }
            "showcolumn" => { let c = arg_u32(a, 0, "column")?; self.set_column_hidden(c, false)?; Ok(self.this()) }
            "hiderow" => { let r = arg_u32(a, 0, "row")?; self.set_row_hidden(r, true)?; Ok(self.this()) }
            "showrow" => { let r = arg_u32(a, 0, "row")?; self.set_row_hidden(r, false)?; Ok(self.this()) }
            "setcellcomment" => {
                // (commentStructOrText, row, column)  — struct may hold {comment/text, author}
                let (text, author) = match a.first() {
                    Some(CfmlValue::Struct(s)) => (
                        s.get_ci("comment").or_else(|| s.get_ci("text")).map(|v| v.as_string()).unwrap_or_default(),
                        s.get_ci("author").map(|v| v.as_string()).unwrap_or_default(),
                    ),
                    Some(v) => (v.as_string(), String::new()),
                    None => (String::new(), String::new()),
                };
                let row = arg_u32(a, 1, "row")?;
                let col = arg_u32(a, 2, "column")?;
                self.set_cell_comment(&text, &author, row, col)?;
                Ok(self.this())
            }
            "setcellhyperlink" => {
                // (link, row, column[, tooltip])
                let link = a.first().map(|v| v.as_string()).unwrap_or_default();
                let row = arg_u32(a, 1, "row")?;
                let col = arg_u32(a, 2, "column")?;
                let tooltip = a.get(3).map(|v| v.as_string()).unwrap_or_default();
                self.set_cell_hyperlink(&link, row, col, &tooltip, None)?;
                Ok(self.this())
            }
            "addautofilter" => {
                let range = a.first().map(|v| v.as_string()).unwrap_or_default();
                self.add_autofilter(&range)?;
                Ok(self.this())
            }
            "addinfo" => {
                if let Some(CfmlValue::Struct(s)) = a.first() {
                    self.add_info(s)?;
                }
                Ok(self.this())
            }
            "addimage" => {
                // (filepathOrBinary, anchor)
                let anchor = a.get(1).map(|v| v.as_string()).unwrap_or_else(|| "A1".to_string());
                match a.first() {
                    Some(CfmlValue::Binary(bytes)) => {
                        self.add_image_bytes(bytes, "image", &anchor)?;
                    }
                    Some(v) => {
                        self.add_image(&v.as_string(), &anchor)?;
                    }
                    None => return Err(CfmlError::runtime("addImage: a file path or binary is required".to_string())),
                }
                Ok(self.this())
            }
            "addchart" => {
                // ({type, series, from, to, title}) — series = array or delimited range list
                if let Some(CfmlValue::Struct(s)) = a.first() {
                    let ct = s.get_ci("type").map(|v| v.as_string()).unwrap_or_else(|| "line".to_string());
                    let series = match s.get_ci("series").or_else(|| s.get_ci("data")) {
                        Some(v) => value_to_cells(&v, ",").iter().map(|c| c.as_string()).collect::<Vec<_>>(),
                        None => Vec::new(),
                    };
                    let from = s.get_ci("from").map(|v| v.as_string()).unwrap_or_else(|| "C1".to_string());
                    let to = s.get_ci("to").map(|v| v.as_string()).unwrap_or_else(|| "H15".to_string());
                    let title = s.get_ci("title").map(|v| v.as_string()).unwrap_or_default();
                    self.add_chart(&ct, &series, &from, &to, &title)?;
                }
                Ok(self.this())
            }
            // ---- fluent data-interchange sources (return this) -------------
            "fromquery" | "fromarray" => {
                let data = a.first().cloned().unwrap_or(CfmlValue::Null);
                let headers = a.get(1).map(|v| v.is_true()).unwrap_or(true);
                self.add_rows(&data, Some(1), 1, headers)?;
                Ok(self.this())
            }
            "fromcsv" => {
                // Replace the active sheet's cells from CSV text.
                let text = a.first().map(|v| v.as_string()).unwrap_or_default();
                let delim = a.get(1).and_then(|v| v.as_string().chars().next()).unwrap_or(',');
                let rows = parse_csv(&text, delim);
                for (ri, row) in rows.iter().enumerate() {
                    for (ci, field) in row.iter().enumerate() {
                        self.set_cell_value(&CfmlValue::string(field.clone()), (ri as u32) + 1, (ci as u32) + 1)?;
                    }
                }
                Ok(self.this())
            }
            // ---- fluent sugar (return this) --------------------------------
            "sheet" => {
                // create-if-absent then select
                let name = a.first().map(|v| v.as_string()).unwrap_or_default();
                if self.set_active_sheet_by_name(&name).is_err() {
                    self.create_sheet(&name)?;
                    self.set_active_sheet_by_name(&name)?;
                }
                Ok(self.this())
            }
            "selectsheet" => {
                match a.first() {
                    Some(CfmlValue::Int(n)) => self.set_active_sheet_number(*n as usize)?,
                    Some(v) => {
                        let s = v.as_string();
                        if let Ok(n) = s.trim().parse::<usize>() {
                            self.set_active_sheet_number(n)?;
                        } else {
                            self.set_active_sheet_by_name(&s)?;
                        }
                    }
                    None => {}
                }
                Ok(self.this())
            }
            "headerrow" => {
                // (array|list) → write to row 1 and bold it
                let data = a.first().cloned().unwrap_or(CfmlValue::Null);
                self.add_row(&data, 1, 1, false, ",")?;
                let bold = CfmlStruct::from_iter([("bold".to_string(), CfmlValue::Bool(true))]);
                self.format_row(&bold, 1)?;
                Ok(self.this())
            }
            "writetocsv" | "writecsv" => {
                let path = a.first().map(|v| v.as_string()).unwrap_or_default();
                let delim = a.get(1).map(|v| v.as_string()).unwrap_or_else(|| ",".to_string());
                std::fs::write(&path, self.to_csv(&delim))
                    .map_err(|e| CfmlError::runtime(format!("Unable to write CSV [{}]: {}", path, e)))?;
                Ok(self.this())
            }
            "addsplitpane" => {
                // (xSplitPosition, ySplitPosition, leftmostColumn, topRow[, activePane])
                let x = a.first().and_then(|v| v.as_string().trim().parse::<f64>().ok()).unwrap_or(0.0);
                let y = a.get(1).and_then(|v| v.as_string().trim().parse::<f64>().ok()).unwrap_or(0.0);
                let left_col = a.get(2).and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(1);
                let top_row = a.get(3).and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(1);
                let active = a.get(4).map(|v| v.as_string()).unwrap_or_else(|| "UPPER_LEFT".to_string());
                self.add_split_pane(x, y, left_col, top_row, &active)?;
                Ok(self.this())
            }
            "setprintorientation" => {
                let mode = a.first().map(|v| v.as_string()).unwrap_or_default();
                self.set_print_orientation(&mode)?;
                Ok(self.this())
            }
            "setfittopage" => {
                let state = a.first().map(|v| v.is_true()).unwrap_or(true);
                let wide = a.get(1).and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(1);
                let high = a.get(2).and_then(|v| v.as_string().trim().parse::<u32>().ok()).unwrap_or(1);
                self.set_fit_to_page(state, wide, high)?;
                Ok(self.this())
            }
            "setheader" => {
                let l = a.first().map(|v| v.as_string()).unwrap_or_default();
                let c = a.get(1).map(|v| v.as_string()).unwrap_or_default();
                let r = a.get(2).map(|v| v.as_string()).unwrap_or_default();
                self.set_header(&l, &c, &r)?;
                Ok(self.this())
            }
            "setfooter" => {
                let l = a.first().map(|v| v.as_string()).unwrap_or_default();
                let c = a.get(1).map(|v| v.as_string()).unwrap_or_default();
                let r = a.get(2).map(|v| v.as_string()).unwrap_or_default();
                self.set_footer(&l, &c, &r)?;
                Ok(self.this())
            }
            "adddatavalidation" => {
                // ({range, type, operator, formula1/value, formula2}) OR positional
                if let Some(CfmlValue::Struct(s)) = a.first() {
                    let range = s.get_ci("range").or_else(|| s.get_ci("cellrange")).map(|v| v.as_string()).unwrap_or_default();
                    let ty = s.get_ci("type").map(|v| v.as_string()).unwrap_or_else(|| "list".to_string());
                    let op = s.get_ci("operator").map(|v| v.as_string()).unwrap_or_default();
                    let f1 = s.get_ci("formula1").or_else(|| s.get_ci("value")).map(|v| v.as_string()).unwrap_or_default();
                    let f2 = s.get_ci("formula2").map(|v| v.as_string()).unwrap_or_default();
                    self.add_data_validation(&range, &ty, &op, &f1, &f2)?;
                }
                Ok(self.this())
            }
            "addconditionalformatting" | "addconditionalformat" => {
                // ({range, operator, value, format})  OR  ({range, type:"colorScale", colors:[…]})
                if let Some(CfmlValue::Struct(s)) = a.first() {
                    let range = s.get_ci("range").or_else(|| s.get_ci("cellrange")).map(|v| v.as_string()).unwrap_or_default();
                    let is_scale = s.get_ci("type").map(|v| v.as_string().eq_ignore_ascii_case("colorScale")).unwrap_or(false);
                    if is_scale {
                        let colors: Vec<String> = match s.get_ci("colors") {
                            Some(v) => value_to_cells(&v, ",").iter().map(|c| c.as_string()).collect(),
                            None => vec!["red".to_string(), "green".to_string()],
                        };
                        self.add_color_scale(&range, &colors)?;
                    } else {
                        let op = s.get_ci("operator").map(|v| v.as_string()).unwrap_or_else(|| "equal".to_string());
                        let value = s.get_ci("value").or_else(|| s.get_ci("formula")).map(|v| v.as_string()).unwrap_or_default();
                        let fmt = match s.get_ci("format") {
                            Some(CfmlValue::Struct(f)) => f,
                            _ => CfmlStruct::empty(),
                        };
                        self.add_conditional_formatting(&range, &op, &value, &fmt)?;
                    }
                }
                Ok(self.this())
            }
            "setactivecell" => {
                let row = arg_u32(a, 0, "row")?;
                let col = arg_u32(a, 1, "column")?;
                self.set_active_cell(row, col)?;
                Ok(self.this())
            }
            "addpagebreaks" => {
                // (rowBreaks, columnBreaks) — each a list/array of indices
                let rows = a.first().map(|v| value_to_cells(v, ",")).unwrap_or_default();
                let cols = a.get(1).map(|v| value_to_cells(v, ",")).unwrap_or_default();
                let rb: Vec<u32> = rows.iter().filter_map(|v| v.as_string().trim().parse().ok()).collect();
                let cb: Vec<u32> = cols.iter().filter_map(|v| v.as_string().trim().parse().ok()).collect();
                self.add_page_breaks(&rb, &cb)?;
                Ok(self.this())
            }
            "setrepeatingrows" => {
                self.set_repeating(&a.first().map(|v| v.as_string()).unwrap_or_default())?;
                Ok(self.this())
            }
            "setrepeatingcolumns" => {
                self.set_repeating(&a.first().map(|v| v.as_string()).unwrap_or_default())?;
                Ok(self.this())
            }
            "load" => {
                self.load(&a.first().map(|v| v.as_string()).unwrap_or_default())?;
                Ok(self.this())
            }
            "fromjson" => {
                let text = a.first().map(|v| v.as_string()).unwrap_or_default();
                let built = CfmlSpreadsheet::from_json_text(&text)?;
                self.book = built.book;
                self.active_sheet = 0;
                Ok(self.this())
            }
            // ---- terminals: return data ------------------------------------
            "getcellcomment" => {
                let row = arg_u32(a, 0, "row")?;
                let col = arg_u32(a, 1, "column")?;
                Ok(self.get_cell_comment(row, col))
            }
            "getcellhyperlink" => {
                let row = arg_u32(a, 0, "row")?;
                let col = arg_u32(a, 1, "column")?;
                Ok(self.get_cell_hyperlink(row, col))
            }
            "readbinary" | "tobinary" => Ok(CfmlValue::Binary(self.to_binary()?)),
            "getcellformula" => {
                let row = arg_u32(a, 0, "row")?;
                let col = arg_u32(a, 1, "column")?;
                Ok(self.get_cell_formula(row, col))
            }
            "getcelltype" => {
                let row = arg_u32(a, 0, "row")?;
                let col = arg_u32(a, 1, "column")?;
                Ok(self.get_cell_type(row, col))
            }
            "iscolumnhidden" => Ok(CfmlValue::Bool(self.is_column_hidden(arg_u32(a, 0, "column")?))),
            "isrowhidden" => Ok(CfmlValue::Bool(self.is_row_hidden(arg_u32(a, 0, "row")?))),
            "getcolumnwidth" => Ok(CfmlValue::Double(self.get_column_width(arg_u32(a, 0, "column")?))),
            "getcellformat" => {
                let row = arg_u32(a, 0, "row")?;
                let col = arg_u32(a, 1, "column")?;
                Ok(self.get_cell_format(row, col))
            }
            "tojson" => {
                let pretty = a.first().map(|v| v.is_true()).unwrap_or(false);
                Ok(CfmlValue::string(self.to_json(pretty)))
            }
            "toquery" => Ok(self.to_query()),
            "toarray" => Ok(self.to_array()),
            "tocsv" => {
                let delim = a.first().map(|v| v.as_string()).unwrap_or_else(|| ",".to_string());
                Ok(CfmlValue::string(self.to_csv(&delim)))
            }
            "getcellvalue" => {
                let row = arg_u32(a, 0, "row")?;
                let col = arg_u32(a, 1, "column")?;
                Ok(self.get_cell_value(row, col))
            }
            "getrowcount" | "rowcount" => Ok(CfmlValue::Int(self.row_count())),
            "getcolumncount" | "columncount" => Ok(CfmlValue::Int(self.column_count())),
            "info" => Ok(self.info_struct()),
            other => Err(CfmlError::runtime(format!(
                "Spreadsheet has no method [{}]",
                other
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Coercion + dispatch (function form). Mirrors crate::image.
// ---------------------------------------------------------------------------

/// Confirm `v` is a Spreadsheet workbook handle and return a cloned `Arc`.
fn coerce_to_workbook(v: &CfmlValue) -> Result<Arc<RwLock<dyn CfmlNative>>, CfmlError> {
    match v {
        CfmlValue::NativeObject(o) => {
            let is_wb = o
                .read()
                .map(|g| g.class_name().eq_ignore_ascii_case("Spreadsheet"))
                .unwrap_or(false);
            if is_wb {
                Ok(Arc::clone(o))
            } else {
                Err(CfmlError::runtime(
                    "Value is a native object but not a Spreadsheet".to_string(),
                ))
            }
        }
        other => Err(CfmlError::runtime(format!(
            "Expected a spreadsheet object, got {}",
            other.type_name()
        ))),
    }
}

/// Lock the workbook handle and forward to `call_method` — the shared impl for
/// every function-form BIF that takes the workbook as its first arg.
fn dispatch(target: &CfmlValue, method: &str, rest: Vec<CfmlValue>) -> CfmlResult {
    let handle = coerce_to_workbook(target)?;
    let mut g = handle
        .write()
        .map_err(|_| CfmlError::runtime("Spreadsheet lock poisoned".to_string()))?;
    g.call_method(method, rest)
}

// ---------------------------------------------------------------------------
// Builtin function entry points (Spreadsheet* BIFs)
// ---------------------------------------------------------------------------

/// `SpreadsheetNew([sheetname][, xmlformat])` — a fresh workbook. `xmlformat`
/// true → `.xlsx` (default), false → legacy `.xls` (writes unsupported).
pub fn fn_spreadsheet_new(args: Vec<CfmlValue>) -> CfmlResult {
    let sheet_name = args.first().map(|v| v.as_string()).filter(|s| !s.is_empty());
    let xmlformat = args.get(1).map(|v| v.is_true()).unwrap_or(true);
    let format = if xmlformat { WorkbookFormat::Xlsx } else { WorkbookFormat::Xls };
    Ok(CfmlSpreadsheet::new_xlsx(sheet_name.as_deref()).into_value(format))
}

/// `Spreadsheet([typeOrPath])` — fluent-builder entry point. For now an alias of
/// `SpreadsheetNew` when given a type ("xls"/"xlsx") or nothing; a path argument
/// (read-then-build) lands in a later phase.
pub fn fn_spreadsheet(args: Vec<CfmlValue>) -> CfmlResult {
    let arg0 = args.first().map(|v| v.as_string()).unwrap_or_default();
    let format = if arg0.eq_ignore_ascii_case("xls") { WorkbookFormat::Xls } else { WorkbookFormat::Xlsx };
    Ok(CfmlSpreadsheet::new_xlsx(None).into_value(format))
}

/// `IsSpreadsheetObject(value)`
pub fn fn_is_spreadsheet_object(args: Vec<CfmlValue>) -> CfmlResult {
    let is = matches!(args.first(), Some(CfmlValue::NativeObject(o))
        if o.read().map(|g| g.class_name().eq_ignore_ascii_case("Spreadsheet")).unwrap_or(false));
    Ok(CfmlValue::Bool(is))
}

macro_rules! bif {
    ($fn_name:ident, $method:literal, $skip:expr) => {
        pub fn $fn_name(args: Vec<CfmlValue>) -> CfmlResult {
            let target = args.first().cloned().unwrap_or(CfmlValue::Null);
            let rest = args.into_iter().skip($skip).collect();
            dispatch(&target, $method, rest)
        }
    };
}

/// `SpreadsheetRead(filepath)` — open an existing `.xlsx` into a fresh workbook.
/// (Lucee/cfsimplicity semantics: returns the workbook. ACF's read-into-object
/// form is handled by the compat CFC.)
pub fn fn_spreadsheet_read(args: Vec<CfmlValue>) -> CfmlResult {
    let path = args.first().map(|v| v.as_string()).unwrap_or_default();
    if path.is_empty() {
        return Err(CfmlError::runtime("SpreadsheetRead: missing file path".to_string()));
    }
    // `.xlsx`/`.xlsm` → umya (full round-trip, styles preserved); legacy
    // `.xls`/`.xlsb`/`.ods` → calamine (data-only).
    let lower = path.to_ascii_lowercase();
    let ss = if lower.ends_with(".xlsx") || lower.ends_with(".xlsm") {
        CfmlSpreadsheet::read_file(&path)?
    } else {
        CfmlSpreadsheet::read_legacy(&path)?
    };
    Ok(ss.into_value(WorkbookFormat::Xlsx))
}

// Function-form wrappers: workbook is arg0, remaining args forwarded.
bif!(fn_spreadsheet_set_cell_value, "setCellValue", 1);
bif!(fn_spreadsheet_get_cell_value, "getCellValue", 1);
bif!(fn_spreadsheet_create_sheet, "createSheet", 1);
bif!(fn_spreadsheet_rename_sheet, "renameSheet", 1);
bif!(fn_spreadsheet_write, "write", 1);
bif!(fn_spreadsheet_info, "info", 1);
bif!(fn_spreadsheet_get_column_count, "getColumnCount", 1);
bif!(fn_spreadsheet_read_binary, "readBinary", 1);
bif!(fn_spreadsheet_set_active_sheet, "setActiveSheet", 1);
bif!(fn_spreadsheet_set_active_sheet_number, "setActiveSheetNumber", 1);
bif!(fn_spreadsheet_add_row, "addRow", 1);
bif!(fn_spreadsheet_add_rows, "addRows", 1);
bif!(fn_spreadsheet_add_column, "addColumn", 1);
bif!(fn_spreadsheet_auto_size_column, "autoSizeColumn", 1);
bif!(fn_spreadsheet_format_cell, "formatCell", 1);
bif!(fn_spreadsheet_format_row, "formatRow", 1);
bif!(fn_spreadsheet_format_column, "formatColumn", 1);
bif!(fn_spreadsheet_format_cell_range, "formatCellRange", 1);
bif!(fn_spreadsheet_merge_cells, "mergeCells", 1);
bif!(fn_spreadsheet_add_freeze_pane, "addFreezePane", 1);
bif!(fn_spreadsheet_set_column_width, "setColumnWidth", 1);
bif!(fn_spreadsheet_set_row_height, "setRowHeight", 1);
bif!(fn_spreadsheet_delete_row, "deleteRow", 1);
bif!(fn_spreadsheet_delete_rows, "deleteRows", 1);
bif!(fn_spreadsheet_delete_column, "deleteColumn", 1);
bif!(fn_spreadsheet_delete_columns, "deleteColumns", 1);
bif!(fn_spreadsheet_shift_rows, "shiftRows", 1);
bif!(fn_spreadsheet_shift_columns, "shiftColumns", 1);
bif!(fn_spreadsheet_set_cell_formula, "setCellFormula", 1);
bif!(fn_spreadsheet_get_cell_formula, "getCellFormula", 1);
bif!(fn_spreadsheet_get_cell_type, "getCellType", 1);
bif!(fn_spreadsheet_clear_cell, "clearCell", 1);
bif!(fn_spreadsheet_clear_cell_range, "clearCellRange", 1);
bif!(fn_spreadsheet_set_cell_range_value, "setCellRangeValue", 1);
bif!(fn_spreadsheet_set_cell_comment, "setCellComment", 1);
bif!(fn_spreadsheet_set_cell_hyperlink, "setCellHyperlink", 1);
bif!(fn_spreadsheet_add_autofilter, "addAutofilter", 1);
bif!(fn_spreadsheet_add_info, "addInfo", 1);
bif!(fn_spreadsheet_add_image, "addImage", 1);
bif!(fn_spreadsheet_add_chart, "addChart", 1);
bif!(fn_spreadsheet_get_cell_comment, "getCellComment", 1);
bif!(fn_spreadsheet_get_cell_hyperlink, "getCellHyperlink", 1);
bif!(fn_spreadsheet_add_split_pane, "addSplitPane", 1);
bif!(fn_spreadsheet_set_print_orientation, "setPrintOrientation", 1);
bif!(fn_spreadsheet_set_fit_to_page, "setFitToPage", 1);
bif!(fn_spreadsheet_set_header, "setHeader", 1);
bif!(fn_spreadsheet_set_footer, "setFooter", 1);
bif!(fn_spreadsheet_set_column_hidden, "setColumnHidden", 1);
bif!(fn_spreadsheet_set_row_hidden, "setRowHidden", 1);
bif!(fn_spreadsheet_add_data_validation, "addDataValidation", 1);
bif!(fn_spreadsheet_add_conditional_formatting, "addConditionalFormatting", 1);
bif!(fn_spreadsheet_get_column_width, "getColumnWidth", 1);
bif!(fn_spreadsheet_get_cell_format, "getCellFormat", 1);
bif!(fn_spreadsheet_set_active_cell, "setActiveCell", 1);
bif!(fn_spreadsheet_add_page_breaks, "addPageBreaks", 1);
bif!(fn_spreadsheet_set_repeating_rows, "setRepeatingRows", 1);
bif!(fn_spreadsheet_set_repeating_columns, "setRepeatingColumns", 1);
bif!(fn_spreadsheet_to_json, "toJson", 1);

/// `SpreadsheetFromJson(json)` — build a workbook from a JSON array.
pub fn fn_spreadsheet_from_json(args: Vec<CfmlValue>) -> CfmlResult {
    let text = args.first().map(|v| v.as_string()).unwrap_or_default();
    Ok(CfmlSpreadsheet::from_json_text(&text)?.into_value(WorkbookFormat::Xlsx))
}

bif!(fn_spreadsheet_to_query, "toQuery", 1);
bif!(fn_spreadsheet_to_array, "toArray", 1);
bif!(fn_spreadsheet_to_csv, "toCsv", 1);
bif!(fn_spreadsheet_write_to_csv, "writeToCsv", 1);

/// `IsSpreadsheetFile(path)` — true if the path looks like a spreadsheet file.
pub fn fn_is_spreadsheet_file(args: Vec<CfmlValue>) -> CfmlResult {
    let p = args.first().map(|v| v.as_string()).unwrap_or_default().to_ascii_lowercase();
    let ok = p.ends_with(".xlsx") || p.ends_with(".xlsm") || p.ends_with(".xls")
        || p.ends_with(".xlsb") || p.ends_with(".ods");
    Ok(CfmlValue::Bool(ok && std::path::Path::new(&args.first().map(|v| v.as_string()).unwrap_or_default()).exists()))
}

/// `SpreadsheetReadCsv(path[, delimiter])` — build a workbook from a CSV file.
pub fn fn_spreadsheet_read_csv(args: Vec<CfmlValue>) -> CfmlResult {
    let path = args.first().map(|v| v.as_string()).unwrap_or_default();
    let delim = args.get(1).and_then(|v| v.as_string().chars().next()).unwrap_or(',');
    let text = std::fs::read_to_string(&path)
        .map_err(|e| CfmlError::runtime(format!("Unable to read CSV [{}]: {}", path, e)))?;
    Ok(CfmlSpreadsheet::from_csv_text(&text, delim).into_value(WorkbookFormat::Xlsx))
}
