//! PDF reading and page rasterisation — the `Pdf*` builtin family.
//!
//! CFML's PDF story has always been a Java one: `<cfpdf>` and `cfdocument` on
//! ACF, PDFBox reached for directly on Lucee. RustCFML has no JVM, so this is
//! native, backed by [`hayro`] — a pure-Rust PDF interpreter tested against
//! 1,400+ files from the PDFBOX and pdf.js regression corpora, and the renderer
//! Typst itself uses for embedded PDFs.
//!
//! # Shape
//!
//! Deliberately the same shape as the spreadsheet family, so the two feel like
//! one engine:
//!
//! * a document is a **shared, reference-typed native object**, so passing it
//!   around costs nothing and every handle sees the same document;
//! * every `Pdf*` function takes the document as its **first argument**;
//! * the same object also carries **methods**, so `Pdf( path ).toImage( 1, 400 )`
//!   chains — mutators would return the document, but a PDF here is read-only,
//!   so every method is terminal.
//!
//! ```cfml
//! doc = PdfRead( expandPath( "/invoice.pdf" ) );
//! info = PdfInfo( doc );                        // { pages, encrypted, pagesizes }
//! img  = PdfToImage( doc, 1, 400 );             // an ordinary image object
//! imageWrite( img, expandPath( "/thumb.png" ) );
//! ```
//!
//! A rendered page is an **image object**, not bytes, so the whole `image*`
//! family applies to it — resize, crop, watermark, write in any format. That is
//! the same choice `imageReadSvg()` makes and it is what makes these compose.
//!
//! # Rendering untrusted input
//!
//! PDF thumbnailing usually means rendering files the public uploaded. hayro is
//! memory-safe Rust with no C in the decode path, which is a far better starting
//! point than PDFBox or Poppler, but a malicious file can still ask for an
//! enormous canvas. [`MAX_RENDER_PIXELS`] caps the output regardless of what the
//! page declares or the caller asks for.

use std::sync::{Arc, Mutex, RwLock, Weak};

use cfml_common::dynamic::{CfmlNative, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf as HayroPdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{RenderCache, RenderSettings, render};

/// Hard ceiling on a rasterised page, in pixels. A PDF page can declare almost
/// any MediaBox, and a caller can ask for almost any width; 40 megapixels is
/// larger than any thumbnail or print preview and small enough that a hostile
/// upload cannot exhaust memory. Exceeded, the scale is reduced to fit rather
/// than the call failing — a slightly smaller image is a better answer than an
/// error for something the caller did not choose.
pub const MAX_RENDER_PIXELS: f32 = 40_000_000.0;

pub struct CfmlPdf {
    /// `Mutex` for the same reason `HtmlDocument` needs one: the parsed
    /// document is `Send` but not `Sync`, and a `CfmlNative` must be both.
    doc: Mutex<HayroPdf>,
    source: String,
    self_ref: Option<Weak<RwLock<CfmlPdf>>>,
}

impl std::fmt::Debug for CfmlPdf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pdf")
            .field("source", &self.source)
            .field("pages", &self.page_count())
            .finish()
    }
}

impl CfmlPdf {
    pub fn parse(bytes: Vec<u8>, source: &str) -> Result<CfmlPdf, CfmlError> {
        let doc = HayroPdf::new(Arc::new(bytes)).map_err(|e| {
            CfmlError::runtime(format!(
                "PdfRead: cannot read {}: {:?}. The file is not a PDF, is truncated, or uses \
                 an encryption this engine cannot open.",
                if source.is_empty() { "the supplied bytes".to_string() } else { format!("'{}'", source) },
                e
            ))
        })?;
        Ok(CfmlPdf { doc: Mutex::new(doc), source: source.to_string(), self_ref: None })
    }

    pub fn into_value(mut self) -> CfmlValue {
        let arc: Arc<RwLock<CfmlPdf>> = Arc::new_cyclic(|weak| {
            self.self_ref = Some(weak.clone());
            RwLock::new(self)
        });
        CfmlValue::NativeObject(arc)
    }

    fn with_doc<R>(&self, f: impl FnOnce(&HayroPdf) -> R) -> R {
        f(&self.doc.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn page_count(&self) -> usize {
        self.with_doc(|d| d.pages().len())
    }

    /// `{ pages, encrypted, pagesizes: [ { width, height } ] }` — page sizes in
    /// PostScript points (1/72"), which is what a PDF natively measures in.
    fn info(&self) -> CfmlValue {
        self.with_doc(|d| {
            let pages = d.pages();
            let mut sizes = Vec::with_capacity(pages.len());
            for p in pages.iter() {
                let (w, h) = p.render_dimensions();
                let mut m = ValueMap::default();
                m.insert("width", CfmlValue::Double(w as f64));
                m.insert("height", CfmlValue::Double(h as f64));
                sizes.push(CfmlValue::strukt(m));
            }
            let mut out = ValueMap::default();
            out.insert("pages", CfmlValue::Int(pages.len() as i64));
            out.insert("source", CfmlValue::string(self.source.clone()));
            out.insert("pagesizes", CfmlValue::array(sizes));
            CfmlValue::strukt(out)
        })
    }

    /// Rasterise one **1-based** page.
    ///
    /// `width` wins if given; otherwise `dpi` (PDF's native resolution is 72dpi,
    /// so dpi/72 is the scale); otherwise 1:1.
    fn render_page(
        &self,
        page_no: usize,
        width: Option<f32>,
        dpi: Option<f32>,
    ) -> Result<image::DynamicImage, CfmlError> {
        self.with_doc(|doc| {
            let pages = doc.pages();
            if page_no < 1 || page_no > pages.len() {
                return Err(CfmlError::runtime(format!(
                    "PdfToImage: page {} is out of range — this document has {} page(s)",
                    page_no,
                    pages.len()
                )));
            }
            let page = &pages[page_no - 1];
            let (pw, ph) = page.render_dimensions();
            if pw <= 0.0 || ph <= 0.0 {
                return Err(CfmlError::runtime(format!(
                    "PdfToImage: page {} declares a zero-sized media box",
                    page_no
                )));
            }

            let mut scale = match (width, dpi) {
                (Some(w), _) if w > 0.0 => w / pw,
                (_, Some(d)) if d > 0.0 => d / 72.0,
                _ => 1.0,
            };
            // Cap the canvas — see MAX_RENDER_PIXELS.
            if pw * scale * ph * scale > MAX_RENDER_PIXELS {
                scale = (MAX_RENDER_PIXELS / (pw * ph)).sqrt();
            }

            let cache = RenderCache::new();
            let pixmap = render(
                page,
                &cache,
                &InterpreterSettings::default(),
                &RenderSettings { x_scale: scale, y_scale: scale, bg_color: WHITE, ..Default::default() },
            );
            let (w, h) = (pixmap.width() as u32, pixmap.height() as u32);
            let data = pixmap.data_as_u8_slice().to_vec();
            image::RgbaImage::from_raw(w, h, data)
                .map(image::DynamicImage::ImageRgba8)
                .ok_or_else(|| {
                    CfmlError::runtime(format!(
                        "PdfToImage: the renderer produced a {}x{} buffer that does not match \
                         its own dimensions",
                        w, h
                    ))
                })
        })
    }
}

impl CfmlNative for CfmlPdf {
    fn method_params(&self, method: &str) -> Option<&'static [&'static str]> {
        Some(match method.to_ascii_lowercase().as_str() {
            "toimage" | "topage" | "render" => &["page", "width", "dpi"][..],
            "info" | "pagecount" | "getpagecount" | "size" | "source" => &[][..],
            _ => return None,
        })
    }

    fn class_name(&self) -> &str {
        "Pdf"
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        let num = |i: usize| -> Option<f32> {
            args.get(i).and_then(|v| {
                let s = v.as_string();
                if s.trim().is_empty() { None } else { s.trim().parse::<f32>().ok() }
            })
        };
        match name.to_ascii_lowercase().as_str() {
            "info" => Ok(self.info()),
            "pagecount" | "getpagecount" | "size" => Ok(CfmlValue::Int(self.page_count() as i64)),
            // toImage( page = 1, width = 0, dpi = 0 )
            "toimage" | "topage" | "render" => {
                let page = num(0).map(|n| n as usize).unwrap_or(1);
                let img = self.render_page(page, num(1), num(2))?;
                Ok(crate::image::image_value_from_dynamic(img))
            }
            "source" => Ok(CfmlValue::string(self.source.clone())),
            other => Err(CfmlError::runtime(format!(
                "Pdf has no method [{}]. Available: info, pageCount, toImage, source.",
                other
            ))),
        }
    }
}

/// The bytes behind a path, a Binary, or an existing Pdf object.
fn source_bytes(v: Option<&CfmlValue>) -> Result<(Vec<u8>, String), CfmlError> {
    match v {
        Some(CfmlValue::Binary(b)) => Ok((b.clone(), String::new())),
        Some(other) => {
            let path = other.as_string();
            if path.trim().is_empty() {
                return Err(CfmlError::runtime(
                    "PdfRead: a file path or PDF binary is required".to_string(),
                ));
            }
            let bytes = std::fs::read(&path).map_err(|e| {
                CfmlError::runtime(format!("PdfRead: cannot read '{}': {}", path, e))
            })?;
            Ok((bytes, path))
        }
        None => Err(CfmlError::runtime(
            "PdfRead: a file path or PDF binary is required".to_string(),
        )),
    }
}

/// `PdfRead( pathOrBinary )` — open a PDF into a document object.
pub fn fn_pdf_read(args: Vec<CfmlValue>) -> CfmlResult {
    let (bytes, source) = source_bytes(args.first())?;
    Ok(CfmlPdf::parse(bytes, &source)?.into_value())
}

/// `Pdf( pathOrBinary )` — the same thing, named for chaining:
/// `Pdf( path ).toImage( 1, 400 )`.
pub fn fn_pdf(args: Vec<CfmlValue>) -> CfmlResult {
    fn_pdf_read(args)
}

/// `IsPdfObject( value )`
pub fn fn_is_pdf_object(args: Vec<CfmlValue>) -> CfmlResult {
    let is = matches!(args.first(), Some(CfmlValue::NativeObject(o))
        if o.read().map(|g| g.class_name() == "Pdf").unwrap_or(false));
    Ok(CfmlValue::Bool(is))
}

/// Call a method on a PDF object passed as argument 0 — the function-form
/// wrapper shared by every `Pdf*` builtin, mirroring the spreadsheet family's
/// `bif!` macro.
fn dispatch(args: Vec<CfmlValue>, method: &str, skip: usize) -> CfmlResult {
    let target = args.first().cloned().unwrap_or(CfmlValue::Null);
    let CfmlValue::NativeObject(obj) = target else {
        return Err(CfmlError::runtime(format!(
            "{}: the first argument must be a PDF object from PdfRead()",
            method
        )));
    };
    let rest: Vec<CfmlValue> = args.into_iter().skip(skip).collect();
    let mut guard = obj
        .write()
        .map_err(|_| CfmlError::runtime("PDF object lock poisoned".to_string()))?;
    if guard.class_name() != "Pdf" {
        return Err(CfmlError::runtime(format!(
            "{}: the first argument must be a PDF object from PdfRead()",
            method
        )));
    }
    guard.call_method(method, rest)
}

pub fn fn_pdf_info(args: Vec<CfmlValue>) -> CfmlResult {
    dispatch(args, "info", 1)
}
pub fn fn_pdf_page_count(args: Vec<CfmlValue>) -> CfmlResult {
    dispatch(args, "pageCount", 1)
}
pub fn fn_pdf_to_image(args: Vec<CfmlValue>) -> CfmlResult {
    dispatch(args, "toImage", 1)
}
