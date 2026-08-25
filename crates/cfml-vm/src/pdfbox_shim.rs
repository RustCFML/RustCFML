//! `org.apache.pdfbox.*` and `java.awt.image.BufferedImage`, over the `Pdf*`
//! builtins.
//!
//! Preside's `NativeImageService` rasterises the first page of an uploaded PDF
//! so the asset pipeline has a thumbnail to work with:
//!
//! ```cfml
//! bufferedImage = createObject( "java", "java.awt.image.BufferedImage" );
//! imageWriter   = createObject( "java", "org.apache.pdfbox.util.PDFImageWriter" );
//! document      = createObject( "java", "org.apache.pdfbox.pdmodel.PDDocument" ).load( filePath );
//! imageWriter.writeImage( document, "jpg", "", "1", "1", prefix, bufferedImage.TYPE_INT_RGB, width );
//! document.close();
//! ```
//!
//! `writeImage` writes one file per page named `<prefix><pageNumber>.<format>`,
//! which is what the caller then picks up — so the adapter's job is to render
//! each page in the range and write it under exactly that name.
//!
//! **The 8th argument is a RESOLUTION, not a width.** PDFBox names it
//! `resolution` and means DPI; Preside passes its target *width* there and then
//! resizes the result down. Honouring the parameter as PDFBox defines it is both
//! more faithful and produces a better thumbnail — the page is rendered at high
//! resolution and downsampled, rather than rendered small and left as-is. The
//! `Pdf*` builtins cap the canvas, so a large value cannot be turned into an
//! enormous allocation.
//!
//! `BufferedImage`'s `TYPE_*` constants are read as **fields**
//! (`bufferedImage.TYPE_INT_RGB`) and are only ever passed straight back into
//! `writeImage`, where they select colour depth. The adapter records them and
//! renders RGB either way: the engine's image objects are RGBA, and writing a
//! JPEG flattens to RGB at encode time regardless.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

pub const PD_DOCUMENT: &str = "org.apache.pdfbox.pdmodel.pddocument";
pub const PDF_IMAGE_WRITER: &str = "org.apache.pdfbox.util.pdfimagewriter";
pub const PDF_RENDERER: &str = "org.apache.pdfbox.rendering.pdfrenderer";
pub const BUFFERED_IMAGE: &str = "java.awt.image.bufferedimage";

pub fn is_pdfbox_class(class_lower: &str) -> bool {
    matches!(
        class_lower,
        PD_DOCUMENT | PDF_IMAGE_WRITER | PDF_RENDERER | BUFFERED_IMAGE
    )
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

pub fn construct(class_lower: &str) -> CfmlResult {
    let mut m = shim(class_lower);
    if class_lower == BUFFERED_IMAGE {
        // Public static int fields, read directly off the instance.
        for (name, v) in [
            ("TYPE_INT_RGB", 1),
            ("TYPE_INT_ARGB", 2),
            ("TYPE_INT_ARGB_PRE", 3),
            ("TYPE_INT_BGR", 4),
            ("TYPE_3BYTE_BGR", 5),
            ("TYPE_4BYTE_ABGR", 6),
            ("TYPE_BYTE_GRAY", 10),
            ("TYPE_BYTE_BINARY", 12),
        ] {
            m.insert(name.to_string(), CfmlValue::Int(v));
        }
    }
    Ok(CfmlValue::strukt(m))
}

fn get(object: &CfmlValue, key: &str) -> Option<CfmlValue> {
    match object {
        CfmlValue::Struct(s) => s.get(key),
        _ => None,
    }
}

fn io_exception(message: impl std::fmt::Display) -> CfmlError {
    CfmlError::new(
        format!("java.io.IOException: {}", message),
        CfmlErrorType::Custom("java.io.IOException".to_string()),
    )
}

fn unsupported(class: &str, method: &str) -> CfmlError {
    CfmlError::new(
        format!(
            "{}.{}() is not supported by RustCFML's PDFBox adapter, which covers reading a \
             document and rasterising its pages (PDDocument.load → PDFImageWriter.writeImage / \
             PDFRenderer.renderImage). PDF *authoring* is not part of it.",
            class, method
        ),
        CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
    )
}

fn arg_num(args: &[CfmlValue], i: usize) -> Option<f64> {
    args.get(i).and_then(|v| {
        let s = v.as_string();
        if s.trim().is_empty() { None } else { s.trim().parse::<f64>().ok() }
    })
}

/// `pdf_read` is `PdfRead()`; `call` invokes a method on the resulting native
/// PDF object; `write_image` is `imageWrite()`.
pub fn dispatch(
    class_lower: &str,
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    pdf_read: &dyn Fn(Vec<CfmlValue>) -> CfmlResult,
    call: &dyn Fn(&CfmlValue, &str, Vec<CfmlValue>) -> CfmlResult,
    write_image: &dyn Fn(Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    match class_lower {
        BUFFERED_IMAGE => match method {
            "init" => construct(BUFFERED_IMAGE),
            // A caller that actually builds a BufferedImage wants a raster to
            // draw on — that is imageNew()'s job, and pretending otherwise
            // would hand back something with no pixels behind it.
            other => Err(unsupported("java.awt.image.BufferedImage", other)),
        },

        PD_DOCUMENT => match method {
            "init" => Ok(CfmlValue::strukt(shim(PD_DOCUMENT))),
            // Static: PDDocument.load( pathOrFileOrBytes )
            "load" => {
                let arg = args.first().cloned().unwrap_or(CfmlValue::Null);
                let source = match &arg {
                    CfmlValue::Binary(_) => arg.clone(),
                    other => {
                        // a java.io.File / FileInputStream knows its own path
                        let p = get(other, "__file_path")
                            .or_else(|| get(other, "__stream_path"))
                            .map(|v| v.as_string())
                            .unwrap_or_else(|| other.as_string());
                        CfmlValue::string(p)
                    }
                };
                let doc = pdf_read(vec![source]).map_err(|e| io_exception(e.message))?;
                let mut m = shim(PD_DOCUMENT);
                m.insert("__doc".to_string(), doc);
                Ok(CfmlValue::strukt(m))
            }
            "getnumberofpages" => {
                let doc = get(object, "__doc").ok_or_else(|| {
                    io_exception("this PDDocument has not been load()ed")
                })?;
                call(&doc, "pageCount", vec![])
            }
            // Nothing to release: the document is a reference-counted native
            // object that goes when the last handle does.
            "close" => Ok(CfmlValue::Null),
            "isencrypted" => Ok(CfmlValue::Bool(false)),
            other => Err(unsupported("org.apache.pdfbox.pdmodel.PDDocument", other)),
        },

        // PDFRenderer is the modern PDFBox 2.x+ API; PDFImageWriter the 1.x one.
        PDF_RENDERER => match method {
            "init" => {
                let mut m = shim(PDF_RENDERER);
                if let Some(doc) = args.first().and_then(|a| get(a, "__doc")) {
                    m.insert("__doc".to_string(), doc);
                }
                Ok(CfmlValue::strukt(m))
            }
            // renderImage( pageIndex [, scale] ) / renderImageWithDPI( pageIndex, dpi )
            "renderimage" | "renderimagewithdpi" => {
                let doc = get(object, "__doc")
                    .ok_or_else(|| io_exception("this PDFRenderer has no document"))?;
                // PDFBox page indices are 0-based; the builtins are 1-based.
                let page = arg_num(&args, 0).unwrap_or(0.0) + 1.0;
                let second = arg_num(&args, 1);
                let (width, dpi) = if method == "renderimagewithdpi" {
                    (0.0, second.unwrap_or(72.0))
                } else {
                    // renderImage's second argument is a SCALE factor.
                    (0.0, second.unwrap_or(1.0) * 72.0)
                };
                call(
                    &doc,
                    "toImage",
                    vec![
                        CfmlValue::Int(page as i64),
                        CfmlValue::Double(width),
                        CfmlValue::Double(dpi),
                    ],
                )
            }
            other => Err(unsupported("org.apache.pdfbox.rendering.PDFRenderer", other)),
        },

        _ => match method {
            "init" => Ok(CfmlValue::strukt(shim(PDF_IMAGE_WRITER))),
            // writeImage( doc, imageFormat, password, startPage, endPage,
            //             filePrefix, imageType, resolution )
            "writeimage" => {
                let doc = args
                    .first()
                    .and_then(|a| get(a, "__doc"))
                    .ok_or_else(|| io_exception("writeImage() needs a loaded PDDocument"))?;
                let format = args
                    .get(1)
                    .map(|v| v.as_string())
                    .filter(|f| !f.trim().is_empty())
                    .unwrap_or_else(|| "jpg".to_string());
                let start = arg_num(&args, 3).unwrap_or(1.0).max(1.0) as i64;
                let end = arg_num(&args, 4).unwrap_or(start as f64).max(start as f64) as i64;
                let prefix = args.get(5).map(|v| v.as_string()).unwrap_or_default();
                if prefix.trim().is_empty() {
                    return Err(io_exception(
                        "writeImage() needs an output file prefix — it names each page \
                         <prefix><pageNumber>.<format>",
                    ));
                }
                // 8th arg is DPI (see the module note), defaulting to PDFBox's own 96.
                let dpi = arg_num(&args, 7).filter(|d| *d > 0.0).unwrap_or(96.0);

                let total = call(&doc, "pageCount", vec![])?
                    .as_string()
                    .trim()
                    .parse::<i64>()
                    .unwrap_or(0);
                let end = end.min(total);
                if start > total {
                    return Err(io_exception(format!(
                        "startPage {} is beyond the document's {} page(s)",
                        start, total
                    )));
                }

                for page in start..=end {
                    let img = call(
                        &doc,
                        "toImage",
                        vec![
                            CfmlValue::Int(page),
                            CfmlValue::Double(0.0),
                            CfmlValue::Double(dpi),
                        ],
                    )?;
                    // PDFBox's naming contract, which the caller reads back.
                    let path = format!("{}{}.{}", prefix, page, format);
                    write_image(vec![
                        img,
                        CfmlValue::string(path),
                        CfmlValue::Null,
                        CfmlValue::Bool(true),
                    ])
                    .map_err(|e| io_exception(e.message))?;
                }
                Ok(CfmlValue::Bool(true))
            }
            other => Err(unsupported("org.apache.pdfbox.util.PDFImageWriter", other)),
        },
    }
}
