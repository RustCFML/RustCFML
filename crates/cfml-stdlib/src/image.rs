//! CFML image support — `imageNew`/`imageRead`/`imageResize`/... and the
//! backing object model for `<cfimage>`.
//!
//! An image is a first-class, mutable CFML object. We model it as a
//! [`CfmlValue::NativeObject`] wrapping [`CfmlImage`], which implements the
//! [`CfmlNative`] trait. That gives us both call forms for free:
//!
//! * member form — `img.resize(w, h)` — dispatched by the VM straight to
//!   [`CfmlImage::call_method`].
//! * function form — `imageResize(img, w, h)` — a plain builtin (see
//!   [`crate::builtins`]) that locks the same `Arc` and forwards to
//!   `call_method`. Because `NativeObject` is a shared handle, the mutation is
//!   visible through the caller's variable, exactly like Lucee.
//!
//! Backed by the pure-Rust `image` crate, so this whole module compiles for
//! `wasm32-unknown-unknown` as well as native. Format is detected from the
//! *content* (magic bytes), never the file extension — matching Lucee's
//! `ImageReadMisnamed` behaviour.

use crate::builtins::{base64_decode_bytes, base64_encode_bytes};
use cfml_common::dynamic::{CfmlNative, CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlResult};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;
use std::sync::{Arc, RwLock};

// Tier 2/3 drawing / transform / filter helpers, all from the pure-Rust
// `imageproc` crate.
use imageproc::drawing::{
    draw_antialiased_line_segment_mut, draw_cubic_bezier_curve_mut, draw_filled_ellipse_mut,
    draw_filled_rect_mut, draw_hollow_ellipse_mut, draw_hollow_rect_mut, draw_line_segment_mut,
    draw_polygon_mut, draw_text_mut, Blend, Canvas,
};
use imageproc::filter::gaussian_blur_f32;
use imageproc::pixelops::interpolate;
use imageproc::geometric_transformations::{
    rotate_about_center_no_crop, warp, Border, Interpolation, Projection,
};
use imageproc::point::Point;
use imageproc::rect::Rect;

/// The bundled default font used by `imageDrawText` / captcha when the caller
/// doesn't (can't) supply one. DejaVu Sans — public-domain-derived Bitstream
/// Vera license (see assets/fonts/LICENSE.DejaVu.txt), permits redistribution.
const DEFAULT_FONT: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");

/// Persistent drawing context — the stateful "graphics" that CFML's
/// `imageSetDrawing*` functions configure and the `imageDraw*` primitives
/// consume. Mirrors the Java2D `Graphics2D` state Lucee/ACF keep on the image.
#[derive(Debug, Clone)]
pub struct DrawingState {
    /// Foreground colour used by the draw primitives (RGB; alpha comes from
    /// `alpha` below so `imageSetDrawingTransparency` composes with it).
    color: [u8; 3],
    /// Background colour used by `imageClearRect` / bevel highlight.
    background: [u8; 3],
    /// Alpha applied to every drawn pixel (255 = opaque). Set via
    /// `imageSetDrawingTransparency(percent)` where percent 0 = opaque.
    alpha: u8,
    /// Stroke width in pixels for the outline primitives.
    stroke_width: f32,
    /// Antialias line/curve primitives when true.
    antialias: bool,
    /// Java2D XOR paint mode (approximated — see docs/known-issues.md §18).
    xor_mode: bool,
}

impl Default for DrawingState {
    fn default() -> Self {
        DrawingState {
            color: [0, 0, 0],
            background: [255, 255, 255],
            alpha: 255,
            stroke_width: 1.0,
            antialias: false,
            xor_mode: false,
        }
    }
}

impl DrawingState {
    /// The current drawing colour as an RGBA pixel, folding in the alpha set by
    /// `imageSetDrawingTransparency`.
    fn rgba(&self) -> image::Rgba<u8> {
        image::Rgba([self.color[0], self.color[1], self.color[2], self.alpha])
    }
}

/// A live, mutable CFML image object.
#[derive(Debug, Clone)]
pub struct CfmlImage {
    /// The decoded pixels.
    img: DynamicImage,
    /// Absolute path of the source file, or "" when created in-memory. Mirrors
    /// Lucee's `imageInfo().source`.
    source: String,
    /// The format the image was read as (drives the default `getBlob`/`write`
    /// encoding when no explicit format/extension is given).
    format: ImageFormat,
    /// Persistent drawing context (Tier 2). Reset to defaults on construction.
    draw: DrawingState,
    /// Original encoded bytes, when known. EXIF/IPTC metadata lives in the
    /// container (JPEG/TIFF), not the decoded pixels, so `imageGetEXIFMetadata`
    /// re-parses these. `None` for in-memory (`imageNew`-created) images; disk
    /// reads fall back to re-reading `source`.
    raw: Option<Vec<u8>>,
}

impl CfmlImage {
    fn new(img: DynamicImage, source: String, format: ImageFormat) -> Self {
        CfmlImage { img, source, format, draw: DrawingState::default(), raw: None }
    }

    /// Attach the original encoded bytes (for later EXIF/IPTC parsing).
    fn with_raw(mut self, bytes: Vec<u8>) -> Self {
        self.raw = Some(bytes);
        self
    }

    /// Wrap in the shared `NativeObject` handle used everywhere images flow
    /// through the VM.
    pub fn into_value(self) -> CfmlValue {
        CfmlValue::NativeObject(Arc::new(RwLock::new(self)))
    }
}

impl CfmlNative for CfmlImage {
    fn class_name(&self) -> &str {
        "Image"
    }

    fn call_method(&mut self, name: &str, args: Vec<CfmlValue>) -> CfmlResult {
        let a = args.as_slice();
        match name.to_lowercase().as_str() {
            "getwidth" => Ok(CfmlValue::Int(self.img.width() as i64)),
            "getheight" => Ok(CfmlValue::Int(self.img.height() as i64)),
            "info" => Ok(self.info_struct()),
            "resize" => {
                self.resize(a)?;
                Ok(CfmlValue::Null)
            }
            "scaletofit" => {
                self.scale_to_fit(a)?;
                // Lucee's member form returns the scaled image (so
                // `x = x.scaleToFit(...)` works); the function form is used for
                // its in-place side effect. We satisfy both: mutate in place
                // AND hand back a fresh handle onto a clone of the result.
                Ok(CfmlImage::new(self.img.clone(), self.source.clone(), self.format).into_value())
            }
            "crop" => {
                self.crop(a)?;
                Ok(CfmlValue::Null)
            }
            "rotate" => {
                self.rotate(a)?;
                Ok(CfmlValue::Null)
            }
            "flip" => {
                self.flip(&arg_str(a, 0, "vertical"))?;
                Ok(CfmlValue::Null)
            }

            // ---- Tier 3: filters -------------------------------------------
            "blur" => { self.blur(a)?; Ok(CfmlValue::Null) }
            "sharpen" => { self.sharpen(a)?; Ok(CfmlValue::Null) }
            "negative" => { self.negative(); Ok(CfmlValue::Null) }
            "grayscale" => { self.grayscale(); Ok(CfmlValue::Null) }
            "makecolortransparent" => { self.make_color_transparent(a)?; Ok(CfmlValue::Null) }
            "maketranslucent" => { self.make_translucent(a)?; Ok(CfmlValue::Null) }

            // ---- Tier 3: geometric transforms ------------------------------
            "translate" | "translatedrawingaxis" => { self.translate(a)?; Ok(CfmlValue::Null) }
            "shear" | "sheardrawingaxis" => { self.shear(a)?; Ok(CfmlValue::Null) }
            "rotatedrawingaxis" => { self.rotate(a)?; Ok(CfmlValue::Null) }

            // ---- Tier 2: drawing state -------------------------------------
            "setdrawingcolor" => { self.draw.color = arg_color(a, 0)?; Ok(CfmlValue::Null) }
            "setbackgroundcolor" => { self.draw.background = arg_color(a, 0)?; Ok(CfmlValue::Null) }
            "setdrawingstroke" => { self.set_stroke(a); Ok(CfmlValue::Null) }
            "setantialiasing" => { self.draw.antialias = arg_on(a, 0, true); Ok(CfmlValue::Null) }
            "setdrawingtransparency" => { self.set_transparency(a); Ok(CfmlValue::Null) }
            "xordrawingmode" => { self.draw.xor_mode = arg_on(a, 0, true); Ok(CfmlValue::Null) }

            // ---- Tier 2: drawing primitives --------------------------------
            "drawline" => { self.draw_line(a)?; Ok(CfmlValue::Null) }
            "drawlines" => { self.draw_lines(a)?; Ok(CfmlValue::Null) }
            "drawpoint" => { self.draw_point(a)?; Ok(CfmlValue::Null) }
            "drawrect" => { self.draw_rect(a)?; Ok(CfmlValue::Null) }
            "drawroundrect" => { self.draw_round_rect(a)?; Ok(CfmlValue::Null) }
            "drawbeveledrect" => { self.draw_beveled_rect(a)?; Ok(CfmlValue::Null) }
            "drawoval" => { self.draw_oval(a)?; Ok(CfmlValue::Null) }
            "drawarc" => { self.draw_arc(a)?; Ok(CfmlValue::Null) }
            "drawcubiccurve" => { self.draw_cubic_curve(a)?; Ok(CfmlValue::Null) }
            "drawquadraticcurve" => { self.draw_quadratic_curve(a)?; Ok(CfmlValue::Null) }
            "drawtext" => { self.draw_text(a)?; Ok(CfmlValue::Null) }
            "clearrect" => { self.clear_rect(a)?; Ok(CfmlValue::Null) }

            // ---- Tier 2: compositing ---------------------------------------
            "drawimage" | "paste" => { self.paste(a)?; Ok(CfmlValue::Null) }
            "overlay" => { self.overlay(a)?; Ok(CfmlValue::Null) }
            "copy" => { self.copy_region(a)?; Ok(CfmlValue::Null) }
            "addborder" => { self.add_border(a)?; Ok(CfmlValue::Null) }

            // ---- Tier 3: metadata ------------------------------------------
            "getexifmetadata" => self.exif_metadata(),
            "getexiftag" => self.exif_tag(&arg_str(a, 0, "")),
            "getiptcmetadata" => self.iptc_metadata(),
            "getiptctag" => self.iptc_tag(&arg_str(a, 0, "")),
            "getbufferedimage" => Err(CfmlError::runtime(
                "imageGetBufferedImage returns a java.awt.BufferedImage, which has no equivalent \
                 in this engine (see docs/known-issues.md §18)"
                    .to_string(),
            )),

            "getblob" => self.get_blob(a.first()),
            "write" => self.write(a),
            "writebase64" => self.write_base64(a),
            other => Err(CfmlError::runtime(format!(
                "Image object has no method [{}]",
                other
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Decoding / construction
// ---------------------------------------------------------------------------

/// Decode raw bytes into a `DynamicImage`, detecting the format from content
/// (NOT any filename). Lucee reads misnamed files correctly, so we must too.
fn decode_bytes(bytes: &[u8]) -> Result<(DynamicImage, ImageFormat), CfmlError> {
    // Decode failures surface as `java.io.IOException` for Lucee/ACF parity:
    // their ImageIO.read throws IOException on non-image bytes, and CFML code
    // (e.g. Preside's NativeImageService) catches that exact type to raise an
    // informative "not an image" error.
    let format = image::guess_format(bytes)
        .map_err(|e| CfmlError::io_exception(format!("Unsupported or unrecognised image format: {}", e)))?;
    let img = image::load_from_memory_with_format(bytes, format)
        .or_else(|_| image::load_from_memory(bytes))
        .map_err(|e| CfmlError::io_exception(format!("Unable to decode image: {}", e)))?;
    Ok((img, format))
}

/// Strip a `data:...;base64,` prefix if present and base64-decode the rest.
fn decode_base64_source(s: &str) -> Vec<u8> {
    let payload = if let Some(idx) = s.find("base64,") {
        &s[idx + "base64,".len()..]
    } else {
        s
    };
    base64_decode_bytes(payload.trim())
}

/// Turn an arbitrary CFML value into a *shared* image handle.
///
/// * an existing image object → the SAME handle (so mutating functions like
///   `imageResize(img, …)` write back through the caller's variable);
/// * `Binary` → freshly decoded;
/// * a `data:` URI or bare base64 String → freshly decoded;
/// * a file path String → read from disk.
///
/// The bare-variable-name String form Lucee also accepts
/// (`imageGetWidth("variables.img")`) is not handled here — that needs VM
/// scope access and is out of scope for the builtin path.
pub fn coerce_to_image(v: &CfmlValue) -> CfmlResult {
    match v {
        CfmlValue::NativeObject(o) => {
            let is_image = o
                .read()
                .map(|g| g.class_name().eq_ignore_ascii_case("Image"))
                .unwrap_or(false);
            if is_image {
                Ok(CfmlValue::NativeObject(Arc::clone(o)))
            } else {
                Err(CfmlError::runtime(
                    "Value is a native object but not an Image".to_string(),
                ))
            }
        }
        CfmlValue::Binary(b) => {
            let (img, format) = decode_bytes(b)?;
            Ok(CfmlImage::new(img, String::new(), format).with_raw(b.clone()).into_value())
        }
        CfmlValue::String(s) => {
            let s = s.as_str();
            // data: URI or something that looks like base64 (no path separators)
            if s.starts_with("data:") {
                let bytes = decode_base64_source(s);
                let (img, format) = decode_bytes(&bytes)?;
                return Ok(CfmlImage::new(img, String::new(), format).with_raw(bytes).into_value());
            }
            read_source_path(s)
        }
        CfmlValue::Null => Err(CfmlError::runtime(
            "Cannot create an image from a null value".to_string(),
        )),
        other => Err(CfmlError::runtime(format!(
            "Cannot create an image from a value of type {}",
            other.type_name()
        ))),
    }
}

/// Read an image from a filesystem path or an http(s) URL.
fn read_source_path(path: &str) -> CfmlResult {
    let bytes = if path.starts_with("http://") || path.starts_with("https://") {
        fetch_url(path)?
    } else {
        std::fs::read(path)
            .map_err(|e| CfmlError::runtime(format!("Unable to read image [{}]: {}", path, e)))?
    };
    let (img, format) = decode_bytes(&bytes)?;
    let source = std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| path.to_string());
    Ok(CfmlImage::new(img, source, format).with_raw(bytes).into_value())
}

#[cfg(feature = "http")]
fn fetch_url(url: &str) -> Result<Vec<u8>, CfmlError> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| CfmlError::runtime(format!("Unable to fetch image URL [{}]: {}", url, e)))?;
    let mut buf = Vec::new();
    std::io::copy(&mut resp.into_reader(), &mut buf)
        .map_err(|e| CfmlError::runtime(format!("Unable to read image URL body: {}", e)))?;
    Ok(buf)
}

#[cfg(not(feature = "http"))]
fn fetch_url(_url: &str) -> Result<Vec<u8>, CfmlError> {
    Err(CfmlError::runtime(
        "Reading an image from a URL requires the 'http' feature".to_string(),
    ))
}

// ---------------------------------------------------------------------------
// Builtin function entry points (function form). Each locks the shared handle
// and forwards to `call_method` so the member form and function form share one
// implementation and one set of semantics.
// ---------------------------------------------------------------------------

fn dispatch(target: &CfmlValue, method: &str, rest: Vec<CfmlValue>) -> CfmlResult {
    let handle = coerce_to_image(target)?;
    if let CfmlValue::NativeObject(o) = &handle {
        let mut g = o
            .write()
            .map_err(|_| CfmlError::runtime("Image lock poisoned".to_string()))?;
        return g.call_method(method, rest);
    }
    unreachable!("coerce_to_image always yields a NativeObject")
}

/// `imageRead(path|url)` — decode a file/URL/image into an image object.
pub fn fn_image_read(args: Vec<CfmlValue>) -> CfmlResult {
    match args.first() {
        Some(v @ CfmlValue::NativeObject(_)) => {
            // imageRead of an existing image → the same shared handle.
            coerce_to_image(v)
        }
        Some(CfmlValue::Binary(b)) => {
            let (img, format) = decode_bytes(b)?;
            Ok(CfmlImage::new(img, String::new(), format).with_raw(b.clone()).into_value())
        }
        Some(CfmlValue::String(s)) => {
            let s = s.as_str();
            if s.starts_with("data:") {
                let bytes = decode_base64_source(s);
                let (img, format) = decode_bytes(&bytes)?;
                Ok(CfmlImage::new(img, String::new(), format).with_raw(bytes).into_value())
            } else {
                read_source_path(s)
            }
        }
        _ => Err(CfmlError::runtime(
            "imageRead requires a path, URL, binary, or image".to_string(),
        )),
    }
}

/// `imageReadBase64(string)` — decode a (optionally data-URI-wrapped) base64
/// string into an image object.
pub fn fn_image_read_base64(args: Vec<CfmlValue>) -> CfmlResult {
    let s = args.first().map(|v| v.as_string()).unwrap_or_default();
    let bytes = decode_base64_source(&s);
    let (img, format) = decode_bytes(&bytes)?;
    Ok(CfmlImage::new(img, String::new(), format).with_raw(bytes).into_value())
}

/// `imageNew([source] [, width] [, height] [, imageType="rgb"] [, canvasColor])`
pub fn fn_image_new(args: Vec<CfmlValue>) -> CfmlResult {
    let has_source = matches!(args.first(), Some(v) if !v.as_string().is_empty() || matches!(v, CfmlValue::NativeObject(_) | CfmlValue::Binary(_)));
    let width = args.get(1).map(|v| v.as_string()).unwrap_or_default();
    let height = args.get(2).map(|v| v.as_string()).unwrap_or_default();
    let has_w = !width.trim().is_empty();
    let has_h = !height.trim().is_empty();

    // Blank-canvas form.
    if has_w || has_h {
        if has_source {
            return Err(CfmlError::runtime(
                "if you define width and height, source has to be empty".to_string(),
            ));
        }
        if has_w != has_h {
            return Err(CfmlError::runtime(
                "missing argument [width or height]; both are required to create a blank image"
                    .to_string(),
            ));
        }
        let w: u32 = width.trim().parse().map_err(|_| {
            CfmlError::runtime(format!("width [{}] is not a valid integer", width))
        })?;
        let h: u32 = height.trim().parse().map_err(|_| {
            CfmlError::runtime(format!("height [{}] is not a valid integer", height))
        })?;
        let img_type = args.get(3).map(|v| v.as_string()).unwrap_or_else(|| "rgb".to_string());
        let canvas = args.get(4).map(|v| v.as_string()).unwrap_or_default();
        return Ok(make_blank(w, h, &img_type, &canvas)?.into_value());
    }

    // Read-from-source form (source given, no dimensions).
    if has_source {
        return fn_image_read(args);
    }

    // Bare imageNew() → a tiny blank RGB canvas (Lucee tolerates this).
    Ok(make_blank(1, 1, "rgb", "")?.into_value())
}

fn make_blank(w: u32, h: u32, img_type: &str, canvas: &str) -> Result<CfmlImage, CfmlError> {
    let color = if canvas.trim().is_empty() {
        None
    } else {
        Some(parse_color(canvas)?)
    };
    let img = match img_type.to_lowercase().as_str() {
        "rgb" => {
            let px = color.map(|c| image::Rgb([c[0], c[1], c[2]])).unwrap_or(image::Rgb([0, 0, 0]));
            DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, px))
        }
        "argb" => {
            let px = color.map(|c| image::Rgba(c)).unwrap_or(image::Rgba([0, 0, 0, 0]));
            DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(w, h, px))
        }
        "gray" | "grayscale" => {
            let lum = color.map(|c| {
                // Rec. 601 luma
                ((0.299 * c[0] as f32) + (0.587 * c[1] as f32) + (0.114 * c[2] as f32)) as u8
            }).unwrap_or(0);
            DynamicImage::ImageLuma8(image::GrayImage::from_pixel(w, h, image::Luma([lum])))
        }
        other => {
            return Err(CfmlError::runtime(format!(
                "imageType has an invalid value [{}], valid values are [rgb,argb,grayscale]",
                other
            )));
        }
    };
    Ok(CfmlImage::new(img, String::new(), ImageFormat::Png))
}

/// `<cfimage action="…" …>` — dispatched from a single options struct.
///
/// `writeToBrowser` is handled in the tag preprocessor (it needs the output
/// buffer), so it never reaches here in the normal static-action case; if a
/// dynamic action lands here we return the `<img>` markup string as a fallback.
pub fn fn_cfimage(args: Vec<CfmlValue>) -> CfmlResult {
    let opts = match args.first() {
        Some(CfmlValue::Struct(s)) => s.clone(),
        _ => {
            return Err(CfmlError::runtime(
                "cfimage expects an attribute struct".to_string(),
            ))
        }
    };
    let get = |k: &str| -> Option<CfmlValue> {
        opts.with_read(|m| {
            m.iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(k))
                .map(|(_, v)| v.clone())
        })
    };
    let get_s = |k: &str| get(k).map(|v| v.as_string()).unwrap_or_default();

    let action = {
        let a = get_s("action");
        if a.is_empty() { "read".to_string() } else { a }
    };
    let source = get("source").unwrap_or(CfmlValue::Null);
    let destination = get_s("destination");
    let is_base64 = get("isBase64").map(|v| v.is_true()).unwrap_or(false);

    match action.to_lowercase().as_str() {
        "read" => {
            if is_base64 {
                fn_image_read_base64(vec![source])
            } else {
                fn_image_read(vec![source])
            }
        }
        "info" => fn_image_info(vec![source]),
        "write" => {
            let quality = get("quality").unwrap_or(CfmlValue::Null);
            let overwrite = get("overwrite").unwrap_or(CfmlValue::Bool(false));
            fn_image_write(vec![source, CfmlValue::string(destination), quality, overwrite])
        }
        "resize" => {
            let handle = coerce_to_image(&source)?;
            let width = get("width").unwrap_or(CfmlValue::Null);
            let height = get("height").unwrap_or(CfmlValue::Null);
            dispatch(&handle, "resize", vec![width, height])?;
            if !destination.is_empty() {
                dispatch(
                    &handle,
                    "write",
                    // (destination, quality, overwrite) — `true` belongs in the
                    // OVERWRITE slot. Passing it positionally as the 2nd argument put
                    // it in `quality`, so `<cfimage action="resize" destination=…>`
                    // died with "quality [true] is not a number" the moment a
                    // destination was given.
                    vec![CfmlValue::string(destination), CfmlValue::Null, CfmlValue::Bool(true)],
                )?;
            }
            Ok(handle)
        }
        "rotate" => {
            let handle = coerce_to_image(&source)?;
            let angle = get("angle").unwrap_or(CfmlValue::Null);
            dispatch(&handle, "rotate", vec![angle])?;
            if !destination.is_empty() {
                dispatch(
                    &handle,
                    "write",
                    // (destination, quality, overwrite) — `true` belongs in the
                    // OVERWRITE slot. Passing it positionally as the 2nd argument put
                    // it in `quality`, so `<cfimage action="resize" destination=…>`
                    // died with "quality [true] is not a number" the moment a
                    // destination was given.
                    vec![CfmlValue::string(destination), CfmlValue::Null, CfmlValue::Bool(true)],
                )?;
            }
            Ok(handle)
        }
        "convert" => {
            let handle = coerce_to_image(&source)?;
            if destination.is_empty() {
                return Err(CfmlError::runtime(
                    "cfimage action=convert requires a destination".to_string(),
                ));
            }
            let overwrite = get("overwrite").unwrap_or(CfmlValue::Bool(false));
            dispatch(&handle, "write", vec![CfmlValue::string(destination), CfmlValue::Null, overwrite])
        }
        "writetobrowser" => {
            // Fallback for the (rare) dynamic-action case: return the markup.
            let blob = dispatch(&source, "getblob", vec![CfmlValue::string("png")])?;
            if let CfmlValue::Binary(bytes) = blob {
                let b64 = base64_encode_bytes(&bytes);
                Ok(CfmlValue::string(format!(
                    "<img src=\"data:image/png;base64,{}\" />",
                    b64
                )))
            } else {
                Ok(CfmlValue::Null)
            }
        }
        "border" => {
            let handle = coerce_to_image(&source)?;
            let thickness = get("thickness").unwrap_or(CfmlValue::Int(1));
            let color = get("color").unwrap_or_else(|| CfmlValue::string("black"));
            dispatch(&handle, "addborder", vec![thickness, color])?;
            if !destination.is_empty() {
                dispatch(
                    &handle,
                    "write",
                    // (destination, quality, overwrite) — `true` belongs in the
                    // OVERWRITE slot. Passing it positionally as the 2nd argument put
                    // it in `quality`, so `<cfimage action="resize" destination=…>`
                    // died with "quality [true] is not a number" the moment a
                    // destination was given.
                    vec![CfmlValue::string(destination), CfmlValue::Null, CfmlValue::Bool(true)],
                )?;
            }
            Ok(handle)
        }
        "captcha" => {
            let text = get_s("text");
            if text.is_empty() {
                return Err(CfmlError::runtime(
                    "cfimage action=captcha requires a text attribute".to_string(),
                ));
            }
            let width: u32 = get("width").map(|v| v.as_string()).and_then(|s| s.trim().parse().ok()).unwrap_or(200);
            let height: u32 = get("height").map(|v| v.as_string()).and_then(|s| s.trim().parse().ok()).unwrap_or(50);
            let handle = make_blank(width, height, "rgb", "white")?.into_value();
            // Black text, sized to the canvas height.
            dispatch(&handle, "setdrawingcolor", vec![CfmlValue::string("black")])?;
            let size = ((height as f32) * 0.6).max(8.0);
            let mut attrs = ValueMap::default();
            attrs.insert("size", CfmlValue::Double(size as f64));
            let tx = 6i64;
            let ty = ((height as f32 - size) / 2.0).max(0.0) as i64;
            dispatch(
                &handle,
                "drawtext",
                vec![
                    CfmlValue::string(text.clone()),
                    CfmlValue::Int(tx),
                    CfmlValue::Int(ty),
                    CfmlValue::strukt(attrs),
                ],
            )?;
            // A few deterministic noise lines (no RNG dependency), seeded off the
            // text so different captchas differ.
            dispatch(&handle, "setdrawingcolor", vec![CfmlValue::string("gray")])?;
            let seed: u32 = text.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
            for k in 0..4u32 {
                let s = seed.wrapping_add(k.wrapping_mul(2654435761));
                let x1 = (s % width) as i64;
                let y1 = ((s >> 8) % height) as i64;
                let x2 = ((s >> 16) % width) as i64;
                let y2 = ((s >> 24) % height) as i64;
                dispatch(
                    &handle,
                    "drawline",
                    vec![CfmlValue::Int(x1), CfmlValue::Int(y1), CfmlValue::Int(x2), CfmlValue::Int(y2)],
                )?;
            }
            if !destination.is_empty() {
                dispatch(
                    &handle,
                    "write",
                    // (destination, quality, overwrite) — `true` belongs in the
                    // OVERWRITE slot. Passing it positionally as the 2nd argument put
                    // it in `quality`, so `<cfimage action="resize" destination=…>`
                    // died with "quality [true] is not a number" the moment a
                    // destination was given.
                    vec![CfmlValue::string(destination), CfmlValue::Null, CfmlValue::Bool(true)],
                )?;
            }
            Ok(handle)
        }
        other => Err(CfmlError::runtime(format!(
            "cfimage: unknown action [{}]",
            other
        ))),
    }
}

/// `isImage(value)` — true for an in-memory image object.
pub fn fn_is_image(args: Vec<CfmlValue>) -> CfmlResult {
    let is = matches!(args.first(), Some(CfmlValue::NativeObject(o))
        if o.read().map(|g| g.class_name().eq_ignore_ascii_case("Image")).unwrap_or(false));
    Ok(CfmlValue::Bool(is))
}

// Thin function-form wrappers -----------------------------------------------

pub fn fn_image_get_width(args: Vec<CfmlValue>) -> CfmlResult {
    dispatch(arg0(&args)?, "getwidth", vec![])
}
pub fn fn_image_get_height(args: Vec<CfmlValue>) -> CfmlResult {
    dispatch(arg0(&args)?, "getheight", vec![])
}
pub fn fn_image_info(args: Vec<CfmlValue>) -> CfmlResult {
    dispatch(arg0(&args)?, "info", vec![])
}
pub fn fn_image_resize(args: Vec<CfmlValue>) -> CfmlResult {
    let (first, rest) = split_first(&args)?;
    dispatch(first, "resize", rest)
}
pub fn fn_image_scale_to_fit(args: Vec<CfmlValue>) -> CfmlResult {
    let (first, rest) = split_first(&args)?;
    dispatch(first, "scaletofit", rest)
}
pub fn fn_image_crop(args: Vec<CfmlValue>) -> CfmlResult {
    let (first, rest) = split_first(&args)?;
    dispatch(first, "crop", rest)
}
pub fn fn_image_rotate(args: Vec<CfmlValue>) -> CfmlResult {
    let (first, rest) = split_first(&args)?;
    dispatch(first, "rotate", rest)
}
pub fn fn_image_flip(args: Vec<CfmlValue>) -> CfmlResult {
    let (first, rest) = split_first(&args)?;
    dispatch(first, "flip", rest)
}
pub fn fn_image_get_blob(args: Vec<CfmlValue>) -> CfmlResult {
    let (first, rest) = split_first(&args)?;
    dispatch(first, "getblob", rest)
}
pub fn fn_image_write(args: Vec<CfmlValue>) -> CfmlResult {
    let (first, rest) = split_first(&args)?;
    dispatch(first, "write", rest)
}
pub fn fn_image_write_base64(args: Vec<CfmlValue>) -> CfmlResult {
    let (first, rest) = split_first(&args)?;
    dispatch(first, "writebase64", rest)
}

fn arg0(args: &[CfmlValue]) -> Result<&CfmlValue, CfmlError> {
    args.first()
        .ok_or_else(|| CfmlError::runtime("image function requires an image argument".to_string()))
}
fn split_first(args: &[CfmlValue]) -> Result<(&CfmlValue, Vec<CfmlValue>), CfmlError> {
    let first = arg0(args)?;
    Ok((first, args.iter().skip(1).cloned().collect()))
}

// ---------------------------------------------------------------------------
// Mutating operations on CfmlImage
// ---------------------------------------------------------------------------

impl CfmlImage {
    /// resize(width, height [, interpolation="highestQuality"] [, blurFactor=1])
    fn resize(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let (cur_w, cur_h) = (self.img.width(), self.img.height());
        let w = resolve_dimension(a.first(), cur_w, "width")?;
        let h = resolve_dimension(a.get(1), cur_h, "height")?;
        let (w, h) = fill_aspect(w, h, cur_w, cur_h)?;
        let filter = interpolation(&arg_str(a, 2, "highestQuality"));
        check_blur_factor(a.get(3))?;
        self.img = self.img.resize_exact(w, h, filter);
        Ok(())
    }

    /// scaleToFit(fitWidth, fitHeight [, interpolation] [, blurFactor]) —
    /// scale into the bounding box, preserving aspect ratio.
    fn scale_to_fit(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let fit_w = dim_or_max(a.first())?;
        let fit_h = dim_or_max(a.get(1))?;
        if fit_w == u32::MAX && fit_h == u32::MAX {
            return Err(CfmlError::runtime(
                "imageScaleToFit requires at least a fitWidth or a fitHeight".to_string(),
            ));
        }
        let filter = interpolation(&arg_str(a, 2, "highestQuality"));
        check_blur_factor(a.get(3))?;
        self.img = self.img.resize(fit_w, fit_h, filter);
        Ok(())
    }

    /// crop(x, y, width, height)
    fn crop(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let x = int_arg(a.first(), "x")? as u32;
        let y = int_arg(a.get(1), "y")? as u32;
        let w = int_arg(a.get(2), "width")? as u32;
        let h = int_arg(a.get(3), "height")? as u32;
        self.img = self.img.crop_imm(x, y, w, h);
        Ok(())
    }

    /// rotate([x, y,] angle [, interpolation]).
    ///
    /// Quarter-turns (multiples of 90°) use the lossless `image` fast paths;
    /// any other angle is rendered with `imageproc`'s projective rotation about
    /// the centre, growing the canvas to fit (matching Lucee/ACF, which enlarge
    /// the drawing surface rather than clipping the corners).
    fn rotate(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        // Lucee's signature is rotate(x, y, angle, interpolation) but the common
        // form is rotate(angle). Detect: if exactly one numeric arg, it's angle.
        let angle = if a.len() == 1 {
            to_f64(a.first())
        } else {
            // x, y, angle[, interp] — angle is the 3rd
            to_f64(a.get(2))
        };
        let norm = ((angle % 360.0) + 360.0) % 360.0;
        self.img = match norm as i64 {
            _ if norm == 0.0 => return Ok(()),
            90 if norm == 90.0 => self.img.rotate90(),
            180 if norm == 180.0 => self.img.rotate180(),
            270 if norm == 270.0 => self.img.rotate270(),
            _ => {
                // Clockwise about centre; theta in radians. Uncovered corners
                // become fully-transparent pixels.
                let rgba = self.img.to_rgba8();
                let theta = norm.to_radians() as f32;
                let interp = interp_kind(&arg_str(a, if a.len() == 1 { 1 } else { 3 }, "bilinear"));
                let out = rotate_about_center_no_crop(
                    &rgba,
                    theta,
                    interp,
                    Border::Constant(Rgba([0, 0, 0, 0])),
                );
                DynamicImage::ImageRgba8(out)
            }
        };
        Ok(())
    }

    /// flip(transpose="vertical") — vertical/horizontal/90/180/270/diagonal/antidiagonal.
    fn flip(&mut self, transpose: &str) -> Result<(), CfmlError> {
        self.img = match transpose.to_lowercase().as_str() {
            "vertical" => self.img.flipv(),
            "horizontal" => self.img.fliph(),
            "90" => self.img.rotate90(),
            "180" => self.img.rotate180(),
            "270" => self.img.rotate270(),
            // main-diagonal transpose = rotate90 then flip horizontally
            "diagonal" => self.img.rotate90().fliph(),
            "antidiagonal" => self.img.rotate270().fliph(),
            other => {
                return Err(CfmlError::runtime(format!(
                    "invalid transpose [{}]; valid values are [vertical,horizontal,diagonal,antidiagonal,90,180,270]",
                    other
                )));
            }
        };
        Ok(())
    }

    // =======================================================================
    // Tier 2/3 — filters, transforms, drawing, compositing, metadata.
    //
    // Drawing runs against an RGBA canvas wrapped in imageproc's `Blend`, so a
    // drawn colour's alpha (from `imageSetDrawingTransparency`) composites over
    // the existing pixels. Outline stroke width is approximated by stamping the
    // primitive over a small disc of offsets. Pixel-exact parity with Lucee's
    // Java2D renderer is not attempted; region/colour parity is (see
    // docs/known-issues.md §18).
    // =======================================================================

    /// Draw against an alpha-blending RGBA canvas, then store the result back.
    fn with_canvas<F: FnOnce(&mut Blend<RgbaImage>)>(&mut self, f: F) {
        let mut canvas = Blend(self.img.to_rgba8());
        f(&mut canvas);
        self.img = DynamicImage::ImageRgba8(canvas.0);
    }

    /// Offsets used to thicken an outline to the current stroke width.
    fn stroke_offsets(&self) -> Vec<(f32, f32)> {
        let r = ((self.draw.stroke_width.max(1.0) - 1.0) / 2.0).round() as i32;
        if r <= 0 {
            return vec![(0.0, 0.0)];
        }
        let mut v = Vec::new();
        for dy in -r..=r {
            for dx in -r..=r {
                if dx * dx + dy * dy <= r * r {
                    v.push((dx as f32, dy as f32));
                }
            }
        }
        v
    }

    // ---- filters ----------------------------------------------------------

    /// blur(radius) — Gaussian blur; `radius` maps to the blur sigma.
    fn blur(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let sigma = {
            let s = arg_str(a, 0, "1");
            s.trim().parse::<f32>().unwrap_or(1.0).max(0.1)
        };
        let rgba = self.img.to_rgba8();
        self.img = DynamicImage::ImageRgba8(gaussian_blur_f32(&rgba, sigma));
        Ok(())
    }

    /// sharpen(gain) — unsharp mask: out = orig + gain·(orig − blurred).
    fn sharpen(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let gain = arg_str(a, 0, "1").trim().parse::<f32>().unwrap_or(1.0).max(0.0);
        let orig = self.img.to_rgba8();
        let blurred = gaussian_blur_f32(&orig, 1.0);
        let mut out = orig.clone();
        for (p_out, (p_o, p_b)) in out.pixels_mut().zip(orig.pixels().zip(blurred.pixels())) {
            for c in 0..3 {
                let v = p_o[c] as f32 + gain * (p_o[c] as f32 - p_b[c] as f32);
                p_out[c] = v.round().clamp(0.0, 255.0) as u8;
            }
            p_out[3] = p_o[3];
        }
        self.img = DynamicImage::ImageRgba8(out);
        Ok(())
    }

    /// negative() — invert RGB, preserving alpha.
    fn negative(&mut self) {
        let mut rgba = self.img.to_rgba8();
        for p in rgba.pixels_mut() {
            p[0] = 255 - p[0];
            p[1] = 255 - p[1];
            p[2] = 255 - p[2];
        }
        self.img = DynamicImage::ImageRgba8(rgba);
    }

    /// grayscale() — Rec. 601 luma, preserving alpha.
    fn grayscale(&mut self) {
        let mut rgba = self.img.to_rgba8();
        for p in rgba.pixels_mut() {
            let l = (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32)
                .round()
                .clamp(0.0, 255.0) as u8;
            p[0] = l;
            p[1] = l;
            p[2] = l;
        }
        self.img = DynamicImage::ImageRgba8(rgba);
    }

    /// makeColorTransparent(color [, tolerance]) — set alpha=0 on pixels whose
    /// RGB is within `tolerance` (0–255 per channel) of `color`.
    fn make_color_transparent(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let target = arg_color(a, 0)?;
        let tol = arg_str(a, 1, "0").trim().parse::<i32>().unwrap_or(0).max(0);
        let mut rgba = self.img.to_rgba8();
        for p in rgba.pixels_mut() {
            if (p[0] as i32 - target[0] as i32).abs() <= tol
                && (p[1] as i32 - target[1] as i32).abs() <= tol
                && (p[2] as i32 - target[2] as i32).abs() <= tol
            {
                p[3] = 0;
            }
        }
        self.img = DynamicImage::ImageRgba8(rgba);
        Ok(())
    }

    /// makeTranslucent(percent) — scale alpha by (1 − percent/100).
    fn make_translucent(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let pct = arg_str(a, 0, "0").trim().parse::<f32>().unwrap_or(0.0).clamp(0.0, 100.0);
        let factor = 1.0 - pct / 100.0;
        let mut rgba = self.img.to_rgba8();
        for p in rgba.pixels_mut() {
            p[3] = (p[3] as f32 * factor).round().clamp(0.0, 255.0) as u8;
        }
        self.img = DynamicImage::ImageRgba8(rgba);
        Ok(())
    }

    // ---- geometric transforms --------------------------------------------

    /// translate(x, y [, interpolation]) — shift the image; uncovered area is
    /// transparent.
    fn translate(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let tx = int_arg(a.first(), "x")? as i32;
        let ty = int_arg(a.get(1), "y")? as i32;
        let rgba = self.img.to_rgba8();
        let out = imageproc::geometric_transformations::translate(
            &rgba,
            (tx, ty),
            Border::Constant(Rgba([0, 0, 0, 0])),
        );
        self.img = DynamicImage::ImageRgba8(out);
        Ok(())
    }

    /// shear(shear, direction="horizontal") — affine shear about the origin.
    fn shear(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let sh = arg_str(a, 0, "0").trim().parse::<f32>().unwrap_or(0.0);
        let dir = arg_str(a, 1, "horizontal").to_lowercase();
        let proj = if dir.starts_with('v') {
            // vertical shear
            Projection::from_matrix([1.0, 0.0, 0.0, sh, 1.0, 0.0, 0.0, 0.0, 1.0])
        } else {
            // horizontal shear
            Projection::from_matrix([1.0, sh, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
        }
        .ok_or_else(|| CfmlError::runtime("invalid shear transform".to_string()))?;
        let rgba = self.img.to_rgba8();
        let out = warp(&rgba, proj, Interpolation::Bilinear, Border::Constant(Rgba([0, 0, 0, 0])));
        self.img = DynamicImage::ImageRgba8(out);
        Ok(())
    }

    // ---- drawing state ----------------------------------------------------

    /// setDrawingStroke(struct|width) — only the line `width` is honoured.
    fn set_stroke(&mut self, a: &[CfmlValue]) {
        let w = match a.first() {
            Some(CfmlValue::Struct(s)) => s.with_read(|m| {
                m.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("width"))
                    .and_then(|(_, v)| v.as_string().trim().parse::<f32>().ok())
            }),
            Some(v) => v.as_string().trim().parse::<f32>().ok(),
            None => None,
        };
        self.draw.stroke_width = w.unwrap_or(1.0).max(1.0);
    }

    /// setDrawingTransparency(percent) — 0 = opaque, 100 = fully transparent.
    fn set_transparency(&mut self, a: &[CfmlValue]) {
        let pct = arg_str(a, 0, "0").trim().parse::<f32>().unwrap_or(0.0).clamp(0.0, 100.0);
        self.draw.alpha = ((1.0 - pct / 100.0) * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    // ---- drawing primitives ----------------------------------------------

    fn draw_line(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let (x1, y1) = (f32_at(a, 0), f32_at(a, 1));
        let (x2, y2) = (f32_at(a, 2), f32_at(a, 3));
        let color = self.draw.rgba();
        let aa = self.draw.antialias;
        let offs = self.stroke_offsets();
        self.with_canvas(|c| {
            for (dx, dy) in &offs {
                stamp_line(c, (x1 + dx, y1 + dy), (x2 + dx, y2 + dy), color, aa);
            }
        });
        Ok(())
    }

    /// drawLines(xcoords, ycoords [, isPolygon=false] [, filled=false]).
    fn draw_lines(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let xs = coord_list(a.first())?;
        let ys = coord_list(a.get(1))?;
        let n = xs.len().min(ys.len());
        if n < 2 {
            return Err(CfmlError::runtime(
                "imageDrawLines requires at least two coordinate pairs".to_string(),
            ));
        }
        let is_polygon = a.get(2).map(|v| v.is_true()).unwrap_or(false);
        let filled = a.get(3).map(|v| v.is_true()).unwrap_or(false);
        let pts: Vec<(f32, f32)> = (0..n).map(|i| (xs[i], ys[i])).collect();
        let color = self.draw.rgba();
        let aa = self.draw.antialias;
        let offs = self.stroke_offsets();
        self.with_canvas(|c| {
            if filled && is_polygon {
                let poly: Vec<Point<i32>> =
                    pts.iter().map(|(x, y)| Point::new(*x as i32, *y as i32)).collect();
                // imageproc requires an open polygon (first != last).
                let poly = dedup_closing(poly);
                if poly.len() >= 3 {
                    draw_polygon_mut(c, &poly, color);
                }
                return;
            }
            for (dx, dy) in &offs {
                for w in pts.windows(2) {
                    stamp_line(c, (w[0].0 + dx, w[0].1 + dy), (w[1].0 + dx, w[1].1 + dy), color, aa);
                }
                if is_polygon {
                    let (a0, b0) = (pts[n - 1], pts[0]);
                    stamp_line(c, (a0.0 + dx, a0.1 + dy), (b0.0 + dx, b0.1 + dy), color, aa);
                }
            }
        });
        Ok(())
    }

    fn draw_point(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let (x, y) = (int_arg(a.first(), "x")? as i32, int_arg(a.get(1), "y")? as i32);
        let color = self.draw.rgba();
        let r = ((self.draw.stroke_width.max(1.0)) / 2.0).round() as i32;
        let (w, h) = (self.img.width() as i32, self.img.height() as i32);
        self.with_canvas(|c| {
            if r <= 0 {
                if x >= 0 && y >= 0 && x < w && y < h {
                    c.draw_pixel(x as u32, y as u32, color);
                }
            } else {
                draw_filled_ellipse_mut(c, (x, y), r, r, color);
            }
        });
        Ok(())
    }

    fn draw_rect(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let (x, y) = (int_arg(a.first(), "x")? as i32, int_arg(a.get(1), "y")? as i32);
        let (w, h) = (int_arg(a.get(2), "width")? as u32, int_arg(a.get(3), "height")? as u32);
        let filled = a.get(4).map(|v| v.is_true()).unwrap_or(false);
        let color = self.draw.rgba();
        let offs = self.stroke_offsets();
        self.with_canvas(|c| {
            let rect = Rect::at(x, y).of_size(w.max(1), h.max(1));
            if filled {
                draw_filled_rect_mut(c, rect, color);
            } else {
                for (dx, dy) in &offs {
                    draw_hollow_rect_mut(
                        c,
                        Rect::at(x + *dx as i32, y + *dy as i32).of_size(w.max(1), h.max(1)),
                        color,
                    );
                }
            }
        });
        Ok(())
    }

    /// drawRoundRect(x, y, width, height, arcWidth, arcHeight [, filled=false]).
    fn draw_round_rect(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let (x, y) = (f32_at(a, 0), f32_at(a, 1));
        let (w, h) = (f32_at(a, 2).max(1.0), f32_at(a, 3).max(1.0));
        let rx = (f32_at(a, 4) / 2.0).clamp(0.0, w / 2.0);
        let ry = (f32_at(a, 5) / 2.0).clamp(0.0, h / 2.0);
        let filled = a.get(6).map(|v| v.is_true()).unwrap_or(false);
        let color = self.draw.rgba();
        let aa = self.draw.antialias;
        let offs = self.stroke_offsets();
        self.with_canvas(|c| {
            if filled {
                // Central cross of two rects + four filled corner ellipses.
                draw_filled_rect_mut(
                    c,
                    Rect::at((x + rx) as i32, y as i32).of_size((w - 2.0 * rx).max(1.0) as u32, h as u32),
                    color,
                );
                draw_filled_rect_mut(
                    c,
                    Rect::at(x as i32, (y + ry) as i32).of_size(w as u32, (h - 2.0 * ry).max(1.0) as u32),
                    color,
                );
                let corners = [
                    (x + rx, y + ry),
                    (x + w - rx, y + ry),
                    (x + rx, y + h - ry),
                    (x + w - rx, y + h - ry),
                ];
                for (cx, cy) in corners {
                    draw_filled_ellipse_mut(c, (cx as i32, cy as i32), rx.max(1.0) as i32, ry.max(1.0) as i32, color);
                }
            } else {
                for (dx, dy) in &offs {
                    let (ox, oy) = (x + dx, y + dy);
                    // straight edges (inset by the corner radius)
                    stamp_line(c, (ox + rx, oy), (ox + w - rx, oy), color, aa);
                    stamp_line(c, (ox + rx, oy + h), (ox + w - rx, oy + h), color, aa);
                    stamp_line(c, (ox, oy + ry), (ox, oy + h - ry), color, aa);
                    stamp_line(c, (ox + w, oy + ry), (ox + w, oy + h - ry), color, aa);
                    // corner quarter-arcs
                    stamp_arc(c, ox + rx, oy + ry, rx, ry, 180.0, 90.0, color, aa);
                    stamp_arc(c, ox + w - rx, oy + ry, rx, ry, 270.0, 90.0, color, aa);
                    stamp_arc(c, ox + w - rx, oy + h - ry, rx, ry, 0.0, 90.0, color, aa);
                    stamp_arc(c, ox + rx, oy + h - ry, rx, ry, 90.0, 90.0, color, aa);
                }
            }
        });
        Ok(())
    }

    /// drawBeveledRect(x, y, width, height [, raised=true] [, filled=false]).
    /// Approximates Java2D's 3D bevel: lit top/left, shaded bottom/right.
    fn draw_beveled_rect(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let (x, y) = (int_arg(a.first(), "x")? as i32, int_arg(a.get(1), "y")? as i32);
        let (w, h) = (int_arg(a.get(2), "width")? as i32, int_arg(a.get(3), "height")? as i32);
        let raised = a.get(4).map(|v| v.is_true()).unwrap_or(true);
        let filled = a.get(5).map(|v| v.is_true()).unwrap_or(false);
        let base = self.draw.color;
        let a8 = self.draw.alpha;
        let lighter = Rgba([
            (base[0] as u16 + 96).min(255) as u8,
            (base[1] as u16 + 96).min(255) as u8,
            (base[2] as u16 + 96).min(255) as u8,
            a8,
        ]);
        let darker = Rgba([
            (base[0] / 2),
            (base[1] / 2),
            (base[2] / 2),
            a8,
        ]);
        let (top_left, bottom_right) = if raised { (lighter, darker) } else { (darker, lighter) };
        let fill = self.draw.rgba();
        self.with_canvas(|c| {
            if filled {
                draw_filled_rect_mut(c, Rect::at(x, y).of_size(w.max(1) as u32, h.max(1) as u32), fill);
            }
            // top & left edges
            stamp_line(c, (x as f32, y as f32), ((x + w) as f32, y as f32), top_left, false);
            stamp_line(c, (x as f32, y as f32), (x as f32, (y + h) as f32), top_left, false);
            // bottom & right edges
            stamp_line(c, (x as f32, (y + h) as f32), ((x + w) as f32, (y + h) as f32), bottom_right, false);
            stamp_line(c, ((x + w) as f32, y as f32), ((x + w) as f32, (y + h) as f32), bottom_right, false);
        });
        Ok(())
    }

    fn draw_oval(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let (x, y) = (f32_at(a, 0), f32_at(a, 1));
        let (w, h) = (f32_at(a, 2).max(1.0), f32_at(a, 3).max(1.0));
        let filled = a.get(4).map(|v| v.is_true()).unwrap_or(false);
        let color = self.draw.rgba();
        let (cx, cy) = ((x + w / 2.0) as i32, (y + h / 2.0) as i32);
        let (rw, rh) = ((w / 2.0).max(1.0) as i32, (h / 2.0).max(1.0) as i32);
        let offs = self.stroke_offsets();
        self.with_canvas(|c| {
            if filled {
                draw_filled_ellipse_mut(c, (cx, cy), rw, rh, color);
            } else {
                for (dx, dy) in &offs {
                    draw_hollow_ellipse_mut(c, (cx + *dx as i32, cy + *dy as i32), rw, rh, color);
                }
            }
        });
        Ok(())
    }

    /// drawArc(x, y, width, height, startAngle, arcAngle [, filled=false]).
    fn draw_arc(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let (x, y) = (f32_at(a, 0), f32_at(a, 1));
        let (w, h) = (f32_at(a, 2).max(1.0), f32_at(a, 3).max(1.0));
        let start = f32_at(a, 4);
        let sweep = f32_at(a, 5);
        let filled = a.get(6).map(|v| v.is_true()).unwrap_or(false);
        let color = self.draw.rgba();
        let aa = self.draw.antialias;
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        let (rx, ry) = (w / 2.0, h / 2.0);
        let offs = self.stroke_offsets();
        self.with_canvas(|c| {
            if filled {
                // pie slice: arc samples + centre, as a filled polygon
                let mut poly: Vec<Point<i32>> = arc_points(cx, cy, rx, ry, start, sweep)
                    .into_iter()
                    .map(|(px, py)| Point::new(px as i32, py as i32))
                    .collect();
                poly.push(Point::new(cx as i32, cy as i32));
                let poly = dedup_closing(poly);
                if poly.len() >= 3 {
                    draw_polygon_mut(c, &poly, color);
                }
            } else {
                for (dx, dy) in &offs {
                    stamp_arc(c, cx + dx, cy + dy, rx, ry, start, sweep, color, aa);
                }
            }
        });
        Ok(())
    }

    /// drawCubicCurve(x1, y1, ctrlx1, ctrly1, ctrlx2, ctrly2, x2, y2).
    fn draw_cubic_curve(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let start = (f32_at(a, 0), f32_at(a, 1));
        let ca = (f32_at(a, 2), f32_at(a, 3));
        let cb = (f32_at(a, 4), f32_at(a, 5));
        let end = (f32_at(a, 6), f32_at(a, 7));
        let color = self.draw.rgba();
        let offs = self.stroke_offsets();
        self.with_canvas(|c| {
            for (dx, dy) in &offs {
                draw_cubic_bezier_curve_mut(
                    c,
                    (start.0 + dx, start.1 + dy),
                    (end.0 + dx, end.1 + dy),
                    (ca.0 + dx, ca.1 + dy),
                    (cb.0 + dx, cb.1 + dy),
                    color,
                );
            }
        });
        Ok(())
    }

    /// drawQuadraticCurve(x1, y1, ctrlx, ctrly, x2, y2) — elevated to a cubic.
    fn draw_quadratic_curve(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let (x1, y1) = (f32_at(a, 0), f32_at(a, 1));
        let (cx, cy) = (f32_at(a, 2), f32_at(a, 3));
        let (x2, y2) = (f32_at(a, 4), f32_at(a, 5));
        // Quadratic → cubic control points.
        let ca = (x1 + 2.0 / 3.0 * (cx - x1), y1 + 2.0 / 3.0 * (cy - y1));
        let cb = (x2 + 2.0 / 3.0 * (cx - x2), y2 + 2.0 / 3.0 * (cy - y2));
        self.draw_cubic_curve(&[
            CfmlValue::Double(x1 as f64), CfmlValue::Double(y1 as f64),
            CfmlValue::Double(ca.0 as f64), CfmlValue::Double(ca.1 as f64),
            CfmlValue::Double(cb.0 as f64), CfmlValue::Double(cb.1 as f64),
            CfmlValue::Double(x2 as f64), CfmlValue::Double(y2 as f64),
        ])
    }

    /// drawText(string, x, y [, attributeCollection{font,size,style}]).
    fn draw_text(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        use ab_glyph::{FontRef, PxScale};
        let text = a.first().map(|v| v.as_string()).unwrap_or_default();
        let x = int_arg(a.get(1), "x")? as i32;
        let y = int_arg(a.get(2), "y")? as i32;
        // Optional attributeCollection: honour `size` (point size). `font`/`style`
        // fall back to the bundled DejaVu Sans (see docs/known-issues.md §18).
        let size = match a.get(3) {
            Some(CfmlValue::Struct(s)) => s.with_read(|m| {
                m.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("size"))
                    .and_then(|(_, v)| v.as_string().trim().parse::<f32>().ok())
            }),
            _ => None,
        }
        .unwrap_or(12.0)
        .max(1.0);
        let font = FontRef::try_from_slice(DEFAULT_FONT)
            .map_err(|e| CfmlError::runtime(format!("bundled font failed to load: {}", e)))?;
        let scale = PxScale::from(size);
        let color = self.draw.rgba();
        self.with_canvas(|c| {
            draw_text_mut(c, color, x, y, scale, &font, &text);
        });
        Ok(())
    }

    /// clearRect(x, y, width, height) — fill with the background colour.
    fn clear_rect(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let (x, y) = (int_arg(a.first(), "x")? as i32, int_arg(a.get(1), "y")? as i32);
        let (w, h) = (int_arg(a.get(2), "width")? as u32, int_arg(a.get(3), "height")? as u32);
        let bg = self.draw.background;
        let color = Rgba([bg[0], bg[1], bg[2], 255]);
        self.with_canvas(|c| {
            draw_filled_rect_mut(c, Rect::at(x, y).of_size(w.max(1), h.max(1)), color);
        });
        Ok(())
    }

    // ---- compositing ------------------------------------------------------

    /// paste / drawImage(source, x, y) — overlay another image at (x, y).
    fn paste(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let src = a.first().ok_or_else(|| CfmlError::runtime("imagePaste requires a source image".to_string()))?;
        let src_rgba = image_rgba_of(src)?;
        let x = int_arg(a.get(1), "x").unwrap_or(0);
        let y = int_arg(a.get(2), "y").unwrap_or(0);
        let mut base = self.img.to_rgba8();
        image::imageops::overlay(&mut base, &src_rgba, x, y);
        self.img = DynamicImage::ImageRgba8(base);
        Ok(())
    }

    /// overlay(image2 [, rule] [, alpha]) — composite image2 over this image.
    /// Only the "over" rule is implemented; `alpha` (0–1) scales image2's alpha.
    fn overlay(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let top = a.first().ok_or_else(|| CfmlError::runtime("imageOverlay requires a second image".to_string()))?;
        let mut top_rgba = image_rgba_of(top)?;
        let alpha = arg_str(a, 2, "1").trim().parse::<f32>().unwrap_or(1.0).clamp(0.0, 1.0);
        if alpha < 1.0 {
            for p in top_rgba.pixels_mut() {
                p[3] = (p[3] as f32 * alpha).round().clamp(0.0, 255.0) as u8;
            }
        }
        let mut base = self.img.to_rgba8();
        image::imageops::overlay(&mut base, &top_rgba, 0, 0);
        self.img = DynamicImage::ImageRgba8(base);
        Ok(())
    }

    /// copy(x, y, width, height [, dx, dy]) — copy a region to (dx, dy).
    fn copy_region(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let x = int_arg(a.first(), "x")? as u32;
        let y = int_arg(a.get(1), "y")? as u32;
        let w = int_arg(a.get(2), "width")? as u32;
        let h = int_arg(a.get(3), "height")? as u32;
        let dx = a.get(4).map(|v| int_arg(Some(v), "dx")).transpose()?.unwrap_or(x as i64);
        let dy = a.get(5).map(|v| int_arg(Some(v), "dy")).transpose()?.unwrap_or(y as i64);
        let mut base = self.img.to_rgba8();
        let region = image::imageops::crop_imm(&base, x, y, w, h).to_image();
        image::imageops::overlay(&mut base, &region, dx, dy);
        self.img = DynamicImage::ImageRgba8(base);
        Ok(())
    }

    /// addBorder(thickness [, color] [, borderType]) — grow the canvas by
    /// `thickness` px on every side, filled with `color` (default the drawing
    /// colour).
    fn add_border(&mut self, a: &[CfmlValue]) -> Result<(), CfmlError> {
        let t = int_arg(a.first(), "thickness").unwrap_or(1).max(0) as u32;
        let color = match a.get(1) {
            Some(v) if !v.as_string().is_empty() => {
                let c = parse_color(&v.as_string())?;
                Rgba(c)
            }
            _ => self.draw.rgba(),
        };
        let src = self.img.to_rgba8();
        let (w, h) = (src.width(), src.height());
        let mut out = RgbaImage::from_pixel(w + 2 * t, h + 2 * t, color);
        image::imageops::overlay(&mut out, &src, t as i64, t as i64);
        self.img = DynamicImage::ImageRgba8(out);
        Ok(())
    }

    // ---- metadata (Tier 3) ------------------------------------------------

    /// The original encoded bytes: cached `raw`, else a re-read of `source`.
    fn container_bytes(&self) -> Option<Vec<u8>> {
        if let Some(b) = &self.raw {
            return Some(b.clone());
        }
        if !self.source.is_empty() {
            return std::fs::read(&self.source).ok();
        }
        None
    }

    /// getEXIFMetadata() — struct of EXIF tag→value pairs (empty if none).
    fn exif_metadata(&self) -> CfmlResult {
        let mut out = ValueMap::default();
        if let Some(bytes) = self.container_bytes() {
            let reader = exif::Reader::new();
            if let Ok(exif) = reader.read_from_container(&mut Cursor::new(&bytes)) {
                for f in exif.fields() {
                    out.insert(
                        f.tag.to_string(),
                        CfmlValue::string(f.display_value().with_unit(&exif).to_string()),
                    );
                }
            }
        }
        Ok(CfmlValue::strukt(out))
    }

    /// getEXIFTag(name) — a single EXIF tag value, "" if absent.
    fn exif_tag(&self, name: &str) -> CfmlResult {
        if let Some(bytes) = self.container_bytes() {
            let reader = exif::Reader::new();
            if let Ok(exif) = reader.read_from_container(&mut Cursor::new(&bytes)) {
                for f in exif.fields() {
                    if f.tag.to_string().eq_ignore_ascii_case(name) {
                        return Ok(CfmlValue::string(
                            f.display_value().with_unit(&exif).to_string(),
                        ));
                    }
                }
            }
        }
        Ok(CfmlValue::string(""))
    }

    /// getIPTCMetadata() — struct of IPTC dataset→value pairs, parsed from the
    /// JPEG APP13 (Photoshop 8BIM / IPTC-NAA) segment. Empty if none.
    fn iptc_metadata(&self) -> CfmlResult {
        let mut out = ValueMap::default();
        if let Some(bytes) = self.container_bytes() {
            // Group repeated datasets (e.g. keywords) into a comma list, in the
            // order first seen — mirroring Lucee.
            let mut order: Vec<String> = Vec::new();
            let mut acc: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
            for (name, value) in parse_iptc(&bytes) {
                if !acc.contains_key(&name) {
                    order.push(name.clone());
                }
                acc.entry(name).or_default().push(value);
            }
            for name in order {
                if let Some(vals) = acc.get(&name) {
                    out.insert(name, CfmlValue::string(vals.join(",")));
                }
            }
        }
        Ok(CfmlValue::strukt(out))
    }

    /// getIPTCTag(name) — a single IPTC value, "" if absent.
    fn iptc_tag(&self, name: &str) -> CfmlResult {
        if let Some(bytes) = self.container_bytes() {
            let mut collected: Vec<String> = Vec::new();
            for (k, v) in parse_iptc(&bytes) {
                if k.eq_ignore_ascii_case(name) {
                    collected.push(v);
                }
            }
            if !collected.is_empty() {
                return Ok(CfmlValue::string(collected.join(",")));
            }
        }
        Ok(CfmlValue::string(""))
    }

    /// getBlob([format]) — raw encoded bytes.
    fn get_blob(&self, format_arg: Option<&CfmlValue>) -> CfmlResult {
        let format = match format_arg.map(|v| v.as_string()) {
            Some(s) if !s.is_empty() => format_from_name(&s)?,
            _ => self.format,
        };
        let bytes = self.encode(format, None)?;
        Ok(CfmlValue::Binary(bytes))
    }

    /// write(destination [, quality=0.75] [, overwrite=true]).
    fn write(&self, a: &[CfmlValue]) -> CfmlResult {
        let dest = a.first().map(|v| v.as_string()).unwrap_or_default();
        if dest.is_empty() {
            return Err(CfmlError::runtime(
                "imageWrite requires a destination path".to_string(),
            ));
        }
        let quality = a.get(1).map(quality_arg).transpose()?;
        let overwrite = a.get(2).map(|v| v.is_true()).unwrap_or(true);
        if !overwrite && std::path::Path::new(&dest).exists() {
            return Err(CfmlError::runtime(format!(
                "destination file [{}] already exists (overwrite is false)",
                dest
            )));
        }
        let format = format_from_path(&dest).unwrap_or(self.format);
        let bytes = self.encode(format, quality)?;
        std::fs::write(&dest, bytes)
            .map_err(|e| CfmlError::runtime(format!("Unable to write image [{}]: {}", dest, e)))?;
        Ok(CfmlValue::Null)
    }

    /// writeBase64(destination, format [, inHTMLFormat=false] [, overwrite=true]).
    fn write_base64(&self, a: &[CfmlValue]) -> CfmlResult {
        let dest = a.first().map(|v| v.as_string()).unwrap_or_default();
        let fmt_name = a.get(1).map(|v| v.as_string()).unwrap_or_else(|| "png".to_string());
        let in_html = a.get(2).map(|v| v.is_true()).unwrap_or(false);
        let format = format_from_name(&fmt_name)?;
        let bytes = self.encode(format, None)?;
        let b64 = base64_encode_bytes(&bytes);
        let out = if in_html {
            format!("data:image/{};base64,{}", fmt_name.to_lowercase(), b64)
        } else {
            b64
        };
        if !dest.is_empty() {
            std::fs::write(&dest, &out).map_err(|e| {
                CfmlError::runtime(format!("Unable to write base64 image [{}]: {}", dest, e))
            })?;
        }
        Ok(CfmlValue::string(out))
    }

    /// Encode the current pixels to the given format, honouring JPEG quality.
    fn encode(&self, format: ImageFormat, quality: Option<f32>) -> Result<Vec<u8>, CfmlError> {
        let mut buf = Cursor::new(Vec::new());
        if format == ImageFormat::Jpeg {
            // JPEG has no alpha; flatten to RGB. Quality 0..1 → 1..100.
            let q = ((quality.unwrap_or(0.75)).clamp(0.0, 1.0) * 100.0).round() as u8;
            let rgb = self.img.to_rgb8();
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, q.max(1));
            enc.encode_image(&rgb)
                .map_err(|e| CfmlError::runtime(format!("JPEG encode failed: {}", e)))?;
        } else {
            self.img
                .write_to(&mut buf, format)
                .map_err(|e| CfmlError::runtime(format!("Image encode failed: {}", e)))?;
        }
        Ok(buf.into_inner())
    }

    /// Build the `imageInfo()` struct — keys and value shapes mirror Lucee's
    /// `Image.java#info()`.
    fn info_struct(&self) -> CfmlValue {
        let (w, h) = (self.img.width(), self.img.height());
        let color = self.img.color();
        let mut info = ValueMap::default();
        info.insert("width", CfmlValue::Int(w as i64));
        info.insert("height", CfmlValue::Int(h as i64));
        info.insert("source", CfmlValue::string(self.source.clone()));
        info.insert("colormodel", color_model_struct(color));
        CfmlValue::strukt(info)
    }
}

// ---------------------------------------------------------------------------
// imageInfo colormodel — matches Lucee key names / value strings.
// ---------------------------------------------------------------------------

fn color_model_struct(color: image::ColorType) -> CfmlValue {
    use image::ColorType::*;
    let (num_color_components, has_alpha, bits_per_channel) = match color {
        L8 => (1, false, 8),
        La8 => (1, true, 8),
        Rgb8 => (3, false, 8),
        Rgba8 => (3, true, 8),
        L16 => (1, false, 16),
        La16 => (1, true, 16),
        Rgb16 => (3, false, 16),
        Rgba16 => (3, true, 16),
        Rgb32F => (3, false, 32),
        Rgba32F => (3, true, 32),
        _ => (3, false, 8),
    };
    let is_gray = matches!(color, L8 | La8 | L16 | La16);
    let num_components = num_color_components + if has_alpha { 1 } else { 0 };
    let pixel_size = num_components * bits_per_channel;

    let mut cm = ValueMap::default();
    cm.insert("alpha_channel_support", CfmlValue::Bool(has_alpha));
    cm.insert("alpha_premultiplied", CfmlValue::Bool(false));
    cm.insert(
        "transparency",
        CfmlValue::string(if has_alpha { "TRANSLUCENT" } else { "OPAQUE" }),
    );
    cm.insert("pixel_size", CfmlValue::Int(pixel_size as i64));
    cm.insert("num_components", CfmlValue::Int(num_components as i64));
    cm.insert(
        "num_color_components",
        CfmlValue::Int(num_color_components as i64),
    );
    cm.insert(
        "colorspace",
        CfmlValue::string(if is_gray {
            "Any of the family of GRAY color spaces"
        } else {
            "Any of the family of RGB color spaces"
        }),
    );
    let mut bits = Vec::new();
    for i in 1..=num_components {
        cm.insert(
            format!("bits_component_{}", i),
            CfmlValue::Int(bits_per_channel as i64),
        );
        bits.push(CfmlValue::Int(bits_per_channel as i64));
    }
    cm.insert("bits_component", CfmlValue::array(bits));
    cm.insert(
        "colormodel_type",
        CfmlValue::string("ComponentColorModel"),
    );
    CfmlValue::strukt(cm)
}

// ---------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------

fn arg_str(a: &[CfmlValue], idx: usize, default: &str) -> String {
    match a.get(idx) {
        Some(v) if !v.as_string().is_empty() => v.as_string(),
        _ => default.to_string(),
    }
}

fn to_f64(v: Option<&CfmlValue>) -> f64 {
    v.map(|v| v.as_string().trim().parse().unwrap_or(0.0)).unwrap_or(0.0)
}

fn int_arg(v: Option<&CfmlValue>, label: &str) -> Result<i64, CfmlError> {
    let s = v.map(|v| v.as_string()).unwrap_or_default();
    s.trim()
        .parse::<f64>()
        .map(|f| f.round() as i64)
        .map_err(|_| CfmlError::runtime(format!("{} [{}] is not a valid number", label, s)))
}

/// Resolve a resize dimension: `"50%"` → percentage of `current`, empty → -1
/// sentinel (aspect-fill), else a positive integer.
fn resolve_dimension(v: Option<&CfmlValue>, current: u32, label: &str) -> Result<i64, CfmlError> {
    let s = v.map(|v| v.as_string()).unwrap_or_default();
    let t = s.trim();
    if t.is_empty() {
        return Ok(-1);
    }
    if let Some(pct) = t.strip_suffix('%') {
        let p: f64 = pct.trim().parse().map_err(|_| {
            CfmlError::runtime(format!("{} [{}] is not a valid percentage", label, s))
        })?;
        return Ok((current as f64 * (p / 100.0)).round() as i64);
    }
    let n: f64 = t
        .parse()
        .map_err(|_| CfmlError::runtime(format!("{} [{}] is not a valid number", label, s)))?;
    if n <= 0.0 {
        return Err(CfmlError::runtime(format!(
            "{} has to be a none negative number",
            label
        )));
    }
    Ok(n.round() as i64)
}

/// Apply Lucee's aspect-preservation rule when exactly one of w/h is the -1
/// sentinel.
fn fill_aspect(w: i64, h: i64, cur_w: u32, cur_h: u32) -> Result<(u32, u32), CfmlError> {
    let (cw, ch) = (cur_w as f64, cur_h as f64);
    let (w, h) = match (w, h) {
        (-1, -1) => {
            return Err(CfmlError::runtime(
                "imageResize requires at least a width or a height".to_string(),
            ))
        }
        (-1, h) => (((cw / ch) * h as f64).round() as i64, h),
        (w, -1) => (w, ((ch / cw) * w as f64).round() as i64),
        (w, h) => (w, h),
    };
    Ok((w.max(1) as u32, h.max(1) as u32))
}

fn dim_or_max(v: Option<&CfmlValue>) -> Result<u32, CfmlError> {
    let s = v.map(|v| v.as_string()).unwrap_or_default();
    let t = s.trim();
    if t.is_empty() {
        return Ok(u32::MAX);
    }
    t.parse::<f64>()
        .map(|f| f.round().max(1.0) as u32)
        .map_err(|_| CfmlError::runtime(format!("dimension [{}] is not a valid number", s)))
}

/// Validate the blurFactor (Lucee requires 0..=10).
fn check_blur_factor(v: Option<&CfmlValue>) -> Result<(), CfmlError> {
    if let Some(v) = v {
        let s = v.as_string();
        if s.trim().is_empty() {
            return Ok(());
        }
        let bf: f64 = s
            .trim()
            .parse()
            .map_err(|_| CfmlError::runtime(format!("blurFactor [{}] is not a number", s)))?;
        if !(0.0..=10.0).contains(&bf) {
            return Err(CfmlError::runtime(
                "blurFactor has to be between 0 and 10".to_string(),
            ));
        }
    }
    Ok(())
}

fn quality_arg(v: &CfmlValue) -> Result<f32, CfmlError> {
    let s = v.as_string();
    if s.trim().is_empty() {
        return Ok(0.75);
    }
    let q: f32 = s
        .trim()
        .parse()
        .map_err(|_| CfmlError::runtime(format!("quality [{}] is not a number", s)))?;
    if !(0.0..=1.0).contains(&q) {
        return Err(CfmlError::runtime(
            "value have to be between 0 and 1".to_string(),
        ));
    }
    Ok(q)
}

/// Map a CFML interpolation name to an `image` resampling filter. Unknown names
/// fall back to the highest quality (Lanczos3) rather than erroring — Lucee
/// accepts a large alias set and the tests iterate the whole list.
fn interpolation(name: &str) -> image::imageops::FilterType {
    use image::imageops::FilterType::*;
    match name.to_lowercase().as_str() {
        "nearest" | "highestperformance" | "highperformance" | "mediumperformance" | "speed" => {
            Nearest
        }
        "bilinear" | "triangle" | "balanced" => Triangle,
        "bicubic" | "cubic" | "catrom" | "mitchell" | "hermite" => CatmullRom,
        "gaussian" | "mediumquality" => Gaussian,
        _ => Lanczos3, // highestQuality / highQuality / lanczos / bessel / …
    }
}

/// Map a CFML interpolation name to imageproc's projective `Interpolation`
/// (used by arbitrary-angle rotate / shear / translate).
fn interp_kind(name: &str) -> Interpolation {
    match name.to_lowercase().as_str() {
        "nearest" | "highestperformance" | "highperformance" | "speed" => Interpolation::Nearest,
        "bicubic" | "cubic" | "highestquality" | "highquality" => Interpolation::Bicubic,
        _ => Interpolation::Bilinear,
    }
}

/// Wrap a decoded image as a CFML image object.
///
/// The one seam other modules need: `pdf.rs` and any future rasteriser produce a
/// `DynamicImage` and must hand back something the whole `image*` family works
/// on, without `CfmlImage`'s fields being public.
pub fn image_value_from_dynamic(img: image::DynamicImage) -> CfmlValue {
    CfmlImage::new(img, String::new(), ImageFormat::Png).into_value()
}

/// `imageReadSvg( source [, width [, height ]] )` → an image object.
///
/// `source` is either a path to an `.svg` file or the SVG markup itself. The
/// result is an ordinary image object, so the whole `image*` family applies to
/// it — resize, crop, draw on, `imageWrite()` to png/jpg/gif. That is the point:
/// SVG becomes a format the rest of CFML can already work with, rather than a
/// one-shot converter.
///
/// * With neither `width` nor `height`, the SVG's own dimensions are used.
/// * With one, the other is derived from the aspect ratio — an SVG is scalable,
///   so distorting it by inventing the missing edge would be a strange default.
/// * With both, the SVG is scaled to fit **inside** that box, preserving aspect
///   ratio and centring the result. Stretching vector art to an arbitrary box is
///   almost never what a caller means by "make it 200x100".
///
/// Rasterisation is `resvg`; fonts come from the operating system's font
/// database, so text in an SVG renders with the same faces the host has. This
/// BIF is native-only (the wasm builds have no font database) — see the `svg`
/// feature in Cargo.toml.
#[cfg(feature = "svg")]
pub fn fn_image_read_svg(args: Vec<CfmlValue>) -> CfmlResult {
    let source = args.first().map(|v| v.as_string()).unwrap_or_default();
    if source.trim().is_empty() {
        return Err(CfmlError::runtime(
            "imageReadSvg: source is required (a path to an .svg file, or SVG markup)".to_string(),
        ));
    }
    let num = |i: usize| -> u32 {
        args.get(i)
            .map(|v| v.as_string().trim().parse::<f64>().unwrap_or(0.0))
            .unwrap_or(0.0)
            .max(0.0) as u32
    };
    let (want_w, want_h) = (num(1), num(2));

    // Markup or a path? Anything containing "<svg" is markup; otherwise read it
    // off disk. Sniffing beats a mode argument here because callers genuinely
    // have both and the two are never confusable.
    let data: Vec<u8> = if source.contains("<svg") || source.trim_start().starts_with("<?xml") {
        source.into_bytes()
    } else {
        std::fs::read(&source).map_err(|e| {
            CfmlError::runtime(format!("imageReadSvg: cannot read '{}': {}", source, e))
        })?
    };

    // The system font database, built once: enumerating fonts is slow enough
    // that doing it per call would dominate a page rendering a set of icons.
    static FONTS: std::sync::OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> =
        std::sync::OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let mut db = resvg::usvg::fontdb::Database::new();
        db.load_system_fonts();
        std::sync::Arc::new(db)
    });

    let mut opt = resvg::usvg::Options::default();
    opt.fontdb = std::sync::Arc::clone(fonts);
    let tree = resvg::usvg::Tree::from_data(&data, &opt).map_err(|e| {
        CfmlError::runtime(format!("imageReadSvg: this is not valid SVG: {}", e))
    })?;

    let native = tree.size();
    let (nw, nh) = (native.width(), native.height());
    if nw <= 0.0 || nh <= 0.0 {
        return Err(CfmlError::runtime(
            "imageReadSvg: the SVG declares a zero or negative size".to_string(),
        ));
    }

    // Scale, then the output canvas. See the doc note for why one-dimension and
    // two-dimension requests behave differently.
    let scale = match (want_w, want_h) {
        (0, 0) => 1.0_f32,
        (w, 0) => w as f32 / nw,
        (0, h) => h as f32 / nh,
        (w, h) => (w as f32 / nw).min(h as f32 / nh),
    };
    let (out_w, out_h) = match (want_w, want_h) {
        (0, 0) => (nw.ceil() as u32, nh.ceil() as u32),
        (w, 0) => (w, (nh * scale).ceil() as u32),
        (0, h) => ((nw * scale).ceil() as u32, h),
        (w, h) => (w, h),
    };
    let (out_w, out_h) = (out_w.clamp(1, 16384), out_h.clamp(1, 16384));

    let mut pixmap = resvg::tiny_skia::Pixmap::new(out_w, out_h).ok_or_else(|| {
        CfmlError::runtime(format!(
            "imageReadSvg: cannot allocate a {}x{} canvas",
            out_w, out_h
        ))
    })?;
    // Centre the scaled art in the canvas when both edges were given.
    let dx = (out_w as f32 - nw * scale) / 2.0;
    let dy = (out_h as f32 - nh * scale) / 2.0;
    let transform = resvg::tiny_skia::Transform::from_translate(dx, dy)
        .pre_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // tiny-skia gives PREMULTIPLIED RGBA; `image` expects straight alpha, so
    // un-premultiply or every semi-transparent pixel comes out too dark.
    let mut rgba = image::RgbaImage::new(out_w, out_h);
    for (i, px) in pixmap.pixels().iter().enumerate() {
        let (x, y) = ((i as u32) % out_w, (i as u32) / out_w);
        rgba.put_pixel(
            x,
            y,
            image::Rgba([px.demultiply().red(), px.demultiply().green(), px.demultiply().blue(), px.alpha()]),
        );
    }

    Ok(CfmlImage::new(
        image::DynamicImage::ImageRgba8(rgba),
        String::new(),
        ImageFormat::Png,
    )
    .into_value())
}

/// `qrCodeGenerate( text [, size [, format [, errorCorrection [, quietZone ]]]] )`
/// → the encoded image as a **Binary**.
///
/// CFML has no QR primitive, so applications reach for a jar —
/// `net.glxn.qrgen`, in Preside's case, which uses it to render the enrolment
/// code an authenticator app scans during two-factor setup. This is that
/// capability natively; the qrgen shim is a thin adapter over it.
///
/// * `size` (default 125) is the target edge length in **pixels**, and the
///   result is square. The module grid rarely divides the requested size
///   exactly, so the grid is drawn at the largest whole-pixel scale that fits
///   and then nearest-neighbour resized to hit `size` on the nose — scaling a QR
///   code with a smoothing filter blurs the module edges and can make it
///   unreadable, which is the one thing this must not do.
/// * `format` (default "png") is any format the image BIFs write: png, gif, jpg,
///   bmp, webp. Preside asks for gif.
/// * `errorCorrection` is L, M (default), Q or H — the proportion of the symbol
///   that can be damaged and still decode. The default matches the qrgen /
///   ZXing default.
/// * `quietZone` (default 4) is the mandatory light margin, in modules. The QR
///   spec requires 4; scanners rely on it, so it is only reducible deliberately.
pub fn fn_qr_code_generate(args: Vec<CfmlValue>) -> CfmlResult {
    use image::{Rgb, RgbImage};
    use qrcode::{EcLevel, QrCode};

    let text = args.first().map(|v| v.as_string()).unwrap_or_default();
    if text.is_empty() {
        return Err(CfmlError::runtime(
            "qrCodeGenerate: text is required — an empty QR code encodes nothing".to_string(),
        ));
    }
    let num = |i: usize, default: i64| -> i64 {
        match args.get(i) {
            Some(CfmlValue::Null) | None => default,
            Some(v) => {
                let s = v.as_string();
                if s.trim().is_empty() { default } else { s.trim().parse().unwrap_or(default) }
            }
        }
    };
    let size = num(1, 125).clamp(16, 4096) as u32;
    let format_name = match args.get(2) {
        Some(CfmlValue::Null) | None => "png".to_string(),
        Some(v) => {
            let s = v.as_string();
            if s.trim().is_empty() { "png".to_string() } else { s }
        }
    };
    let format = format_from_name(&format_name)?;
    let ec = match args
        .get(3)
        .map(|v| v.as_string().trim().to_ascii_uppercase())
        .unwrap_or_default()
        .as_str()
    {
        "L" => EcLevel::L,
        "Q" => EcLevel::Q,
        "H" => EcLevel::H,
        // "" and "M" alike: the qrgen/ZXing default.
        _ => EcLevel::M,
    };
    let quiet = num(4, 4).clamp(0, 32) as u32;

    let code = QrCode::with_error_correction_level(text.as_bytes(), ec).map_err(|e| {
        CfmlError::runtime(format!(
            "qrCodeGenerate: cannot encode this text as a QR code: {}. The most likely cause              is that it is too long for the chosen error-correction level — try \"L\".",
            e
        ))
    })?;

    let modules = code.width() as u32;
    let grid = modules + quiet * 2;
    // Largest whole-pixel module that still fits inside the requested size, so
    // the grid stays crisp; at least 1 so a tiny `size` still produces a symbol.
    let scale = (size / grid).max(1);
    let drawn = grid * scale;

    let colors = code.to_colors();
    let mut img = RgbImage::from_pixel(drawn, drawn, Rgb([255, 255, 255]));
    for (i, c) in colors.iter().enumerate() {
        if *c != qrcode::Color::Dark {
            continue;
        }
        let mx = (i as u32) % modules;
        let my = (i as u32) / modules;
        let x0 = (mx + quiet) * scale;
        let y0 = (my + quiet) * scale;
        for y in y0..y0 + scale {
            for x in x0..x0 + scale {
                img.put_pixel(x, y, Rgb([0, 0, 0]));
            }
        }
    }

    let mut dynamic = image::DynamicImage::ImageRgb8(img);
    if drawn != size {
        // Nearest, never a smoothing filter — see the doc note.
        dynamic = dynamic.resize_exact(size, size, image::imageops::FilterType::Nearest);
    }

    let encoded = CfmlImage::new(dynamic, String::new(), format).encode(format, None)?;
    Ok(CfmlValue::Binary(encoded))
}

fn format_from_name(name: &str) -> Result<ImageFormat, CfmlError> {
    match name.trim().trim_start_matches('.').to_lowercase().as_str() {
        "png" => Ok(ImageFormat::Png),
        "jpg" | "jpeg" => Ok(ImageFormat::Jpeg),
        "gif" => Ok(ImageFormat::Gif),
        "bmp" => Ok(ImageFormat::Bmp),
        "tif" | "tiff" => Ok(ImageFormat::Tiff),
        "webp" => Ok(ImageFormat::WebP),
        "ico" => Ok(ImageFormat::Ico),
        other => Err(CfmlError::runtime(format!(
            "Unsupported image format [{}]",
            other
        ))),
    }
}

fn format_from_path(path: &str) -> Option<ImageFormat> {
    let ext = std::path::Path::new(path).extension()?.to_str()?;
    format_from_name(ext).ok()
}

/// Parse a CFML color spec: hex (`FF0000`/`#FF0000`), an `r,g,b` list, or a
/// small set of AWT/CSS named colors. Returns RGBA (opaque).
pub fn parse_color(spec: &str) -> Result<[u8; 4], CfmlError> {
    let s = spec.trim();
    // RGB list: "255,0,0"
    if s.contains(',') {
        let parts: Vec<&str> = s.split(',').map(|p| p.trim()).collect();
        if parts.len() == 3 {
            let r = parts[0].parse::<u8>();
            let g = parts[1].parse::<u8>();
            let b = parts[2].parse::<u8>();
            if let (Ok(r), Ok(g), Ok(b)) = (r, g, b) {
                return Ok([r, g, b, 255]);
            }
        }
    }
    // Hex
    let hex = s.trim_start_matches('#');
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
        return Ok([r, g, b, 255]);
    }
    // Named (extended below with the AWT set used by the drawing tier)
    let named = match s.to_lowercase().as_str() {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "cyan" => [0, 255, 255],
        "magenta" => [255, 0, 255],
        "yellow" => [255, 255, 0],
        "gray" | "grey" => [128, 128, 128],
        "darkgray" | "darkgrey" => [64, 64, 64],
        "lightgray" | "lightgrey" => [192, 192, 192],
        "orange" => [255, 200, 0],
        "pink" => [255, 175, 175],
        other => {
            return Err(CfmlError::runtime(format!(
                "invalid color [{}]",
                other
            )))
        }
    };
    Ok([named[0], named[1], named[2], 255])
}

// ===========================================================================
// Tier 2/3 free helpers
// ===========================================================================

/// Parse the numeric argument at `idx`, defaulting to 0.
fn f32_at(a: &[CfmlValue], idx: usize) -> f32 {
    a.get(idx)
        .map(|v| v.as_string().trim().parse::<f32>().unwrap_or(0.0))
        .unwrap_or(0.0)
}

/// Parse a colour argument (hex / r,g,b / named) at `idx`, dropping alpha.
fn arg_color(a: &[CfmlValue], idx: usize) -> Result<[u8; 3], CfmlError> {
    let s = a.get(idx).map(|v| v.as_string()).unwrap_or_default();
    let c = parse_color(&s)?;
    Ok([c[0], c[1], c[2]])
}

/// Parse an on/off flag ("on"/"off"/"yes"/"no"/true/false/1/0).
fn arg_on(a: &[CfmlValue], idx: usize, default: bool) -> bool {
    match a.get(idx) {
        Some(CfmlValue::Bool(b)) => *b,
        Some(v) => match v.as_string().trim().to_lowercase().as_str() {
            "on" | "yes" | "true" | "1" => true,
            "off" | "no" | "false" | "0" => false,
            "" => default,
            _ => default,
        },
        None => default,
    }
}

/// Read a coordinate list — a CFML array of numbers or a comma-delimited string.
fn coord_list(v: Option<&CfmlValue>) -> Result<Vec<f32>, CfmlError> {
    match v {
        Some(CfmlValue::Array(items)) => Ok(items
            .iter()
            .map(|x| x.as_string().trim().parse::<f32>().unwrap_or(0.0))
            .collect()),
        Some(other) => {
            let s = other.as_string();
            if s.trim().is_empty() {
                return Ok(vec![]);
            }
            Ok(s.split(',')
                .map(|p| p.trim().parse::<f32>().unwrap_or(0.0))
                .collect())
        }
        None => Ok(vec![]),
    }
}

/// Drop a trailing point equal to the first (imageproc polygons must be open).
fn dedup_closing(mut poly: Vec<Point<i32>>) -> Vec<Point<i32>> {
    while poly.len() >= 2 && poly.first() == poly.last() {
        poly.pop();
    }
    poly
}

/// Draw one line segment on the blend canvas, antialiased when requested.
fn stamp_line(
    c: &mut Blend<RgbaImage>,
    start: (f32, f32),
    end: (f32, f32),
    color: Rgba<u8>,
    aa: bool,
) {
    if aa {
        // The antialiased routine needs a raw `GenericImage`, not the Blend
        // canvas wrapper — draw straight onto the inner buffer.
        draw_antialiased_line_segment_mut(
            &mut c.0,
            (start.0.round() as i32, start.1.round() as i32),
            (end.0.round() as i32, end.1.round() as i32),
            color,
            interpolate,
        );
    } else {
        draw_line_segment_mut(c, start, end, color);
    }
}

/// Sample an elliptical arc (Java `drawArc` convention: degrees, 0° at 3
/// o'clock, positive angles counter-clockwise) into a polyline.
fn arc_points(cx: f32, cy: f32, rx: f32, ry: f32, start_deg: f32, sweep_deg: f32) -> Vec<(f32, f32)> {
    let steps = ((sweep_deg.abs() / 4.0).ceil() as usize).max(2);
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = start_deg + sweep_deg * (i as f32 / steps as f32);
        let rad = t.to_radians();
        // screen y grows downward, so counter-clockwise ⇒ subtract the sine term
        pts.push((cx + rx * rad.cos(), cy - ry * rad.sin()));
    }
    pts
}

/// Draw an elliptical arc as connected segments.
fn stamp_arc(
    c: &mut Blend<RgbaImage>,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    start_deg: f32,
    sweep_deg: f32,
    color: Rgba<u8>,
    aa: bool,
) {
    let pts = arc_points(cx, cy, rx, ry, start_deg, sweep_deg);
    for w in pts.windows(2) {
        stamp_line(c, w[0], w[1], color, aa);
    }
}

/// Coerce any image-ish value (handle / path / binary / base64) to an owned
/// RGBA buffer, via the shared `getBlob` round-trip (avoids a downcast).
fn image_rgba_of(v: &CfmlValue) -> Result<RgbaImage, CfmlError> {
    let blob = dispatch(v, "getblob", vec![CfmlValue::string("png")])?;
    if let CfmlValue::Binary(bytes) = blob {
        let (img, _) = decode_bytes(&bytes)?;
        Ok(img.to_rgba8())
    } else {
        Err(CfmlError::runtime(
            "unable to read the source image for compositing".to_string(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Minimal IPTC (IIM) reader — walks the JPEG APP13 "Photoshop 3.0" 8BIM blocks
// to the IPTC-NAA (0x0404) resource, then decodes the 0x1C dataset records.
// Only the common editorial datasets are named; everything else is skipped.
// ---------------------------------------------------------------------------

/// Map an IPTC record:dataset pair to a Lucee-style human key.
fn iptc_key(record: u8, dataset: u8) -> Option<&'static str> {
    match (record, dataset) {
        (2, 5) => Some("object_name"),      // title
        (2, 25) => Some("keywords"),
        (2, 40) => Some("special_instructions"),
        (2, 55) => Some("date_created"),
        (2, 80) => Some("by_line"),         // author/creator
        (2, 85) => Some("by_line_title"),
        (2, 90) => Some("city"),
        (2, 95) => Some("province_state"),
        (2, 101) => Some("country_name"),
        (2, 105) => Some("headline"),
        (2, 110) => Some("credit"),
        (2, 115) => Some("source"),
        (2, 116) => Some("copyright_notice"),
        (2, 120) => Some("caption"),        // description
        (2, 122) => Some("caption_writer"),
        _ => None,
    }
}

/// Parse IPTC datasets from JPEG bytes. Returns (key, value) pairs in file
/// order. Non-JPEG input, or JPEG without an APP13/IPTC segment, yields none.
fn parse_iptc(bytes: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // JPEG SOI
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return out;
    }
    let mut i = 2usize;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            break;
        }
        let marker = bytes[i + 1];
        // Standalone markers (RSTn, SOI/EOI) have no length.
        if marker == 0xD9 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        let seg_len = ((bytes[i + 2] as usize) << 8) | bytes[i + 3] as usize;
        if seg_len < 2 || i + 2 + seg_len > bytes.len() {
            break;
        }
        let payload = &bytes[i + 4..i + 2 + seg_len];
        if marker == 0xED {
            // APP13 — look for "Photoshop 3.0\0" then walk 8BIM resources.
            if let Some(iptc) = extract_iptc_from_app13(payload) {
                out.extend(parse_iim(iptc));
            }
        }
        // Stop once we reach compressed scan data.
        if marker == 0xDA {
            break;
        }
        i += 2 + seg_len;
    }
    out
}

/// Locate the IPTC-NAA (0x0404) 8BIM resource inside an APP13 payload.
fn extract_iptc_from_app13(payload: &[u8]) -> Option<&[u8]> {
    let sig = b"Photoshop 3.0\0";
    let start = payload.windows(sig.len()).position(|w| w == sig)? + sig.len();
    let mut p = start;
    while p + 12 <= payload.len() {
        if &payload[p..p + 4] != b"8BIM" {
            break;
        }
        let id = ((payload[p + 4] as u16) << 8) | payload[p + 5] as u16;
        // Pascal-style name, padded to an even length.
        let name_len = payload[p + 6] as usize;
        let mut q = p + 7 + name_len;
        if (name_len + 1) % 2 != 0 {
            q += 1; // pad byte
        }
        if q + 4 > payload.len() {
            break;
        }
        let size = ((payload[q] as usize) << 24)
            | ((payload[q + 1] as usize) << 16)
            | ((payload[q + 2] as usize) << 8)
            | payload[q + 3] as usize;
        q += 4;
        let end = (q + size).min(payload.len());
        if id == 0x0404 {
            return Some(&payload[q..end]);
        }
        // Data is padded to an even length.
        p = end + (size % 2);
    }
    None
}

/// Decode IPTC IIM 0x1C dataset records into (key, value) pairs.
fn parse_iim(data: &[u8]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 5 <= data.len() {
        if data[i] != 0x1C {
            i += 1;
            continue;
        }
        let record = data[i + 1];
        let dataset = data[i + 2];
        let len = ((data[i + 3] as usize) << 8) | data[i + 4] as usize;
        let vstart = i + 5;
        let vend = (vstart + len).min(data.len());
        if vstart > data.len() {
            break;
        }
        if let Some(key) = iptc_key(record, dataset) {
            let value = String::from_utf8_lossy(&data[vstart..vend]).trim().to_string();
            out.push((key.to_string(), value));
        }
        i = vend;
    }
    out
}

// ===========================================================================
// Function-form wrappers (Tier 2/3). Each locks the shared handle and forwards
// to `call_method`, so member form and function form share one implementation.
// ===========================================================================

macro_rules! image_fn {
    ($name:ident, $method:literal) => {
        pub fn $name(args: Vec<CfmlValue>) -> CfmlResult {
            let (first, rest) = split_first(&args)?;
            dispatch(first, $method, rest)
        }
    };
}

// filters
image_fn!(fn_image_blur, "blur");
image_fn!(fn_image_sharpen, "sharpen");
image_fn!(fn_image_negative, "negative");
image_fn!(fn_image_grayscale, "grayscale");
image_fn!(fn_image_make_color_transparent, "makecolortransparent");
image_fn!(fn_image_make_translucent, "maketranslucent");
// transforms
image_fn!(fn_image_translate, "translate");
image_fn!(fn_image_translate_drawing_axis, "translatedrawingaxis");
image_fn!(fn_image_shear, "shear");
image_fn!(fn_image_shear_drawing_axis, "sheardrawingaxis");
image_fn!(fn_image_rotate_drawing_axis, "rotatedrawingaxis");
// drawing state
image_fn!(fn_image_set_drawing_color, "setdrawingcolor");
image_fn!(fn_image_set_background_color, "setbackgroundcolor");
image_fn!(fn_image_set_drawing_stroke, "setdrawingstroke");
image_fn!(fn_image_set_antialiasing, "setantialiasing");
image_fn!(fn_image_set_drawing_transparency, "setdrawingtransparency");
image_fn!(fn_image_xor_drawing_mode, "xordrawingmode");
// primitives
image_fn!(fn_image_draw_line, "drawline");
image_fn!(fn_image_draw_lines, "drawlines");
image_fn!(fn_image_draw_point, "drawpoint");
image_fn!(fn_image_draw_rect, "drawrect");
image_fn!(fn_image_draw_round_rect, "drawroundrect");
image_fn!(fn_image_draw_beveled_rect, "drawbeveledrect");
image_fn!(fn_image_draw_oval, "drawoval");
image_fn!(fn_image_draw_arc, "drawarc");
image_fn!(fn_image_draw_cubic_curve, "drawcubiccurve");
image_fn!(fn_image_draw_quadratic_curve, "drawquadraticcurve");
image_fn!(fn_image_draw_text, "drawtext");
image_fn!(fn_image_clear_rect, "clearrect");
// compositing
image_fn!(fn_image_draw_image, "drawimage");
image_fn!(fn_image_paste, "paste");
image_fn!(fn_image_overlay, "overlay");
image_fn!(fn_image_copy, "copy");
image_fn!(fn_image_add_border, "addborder");
// metadata
image_fn!(fn_image_get_exif_metadata, "getexifmetadata");
image_fn!(fn_image_get_exif_tag, "getexiftag");
image_fn!(fn_image_get_iptc_metadata, "getiptcmetadata");
image_fn!(fn_image_get_iptc_tag, "getiptctag");
image_fn!(fn_image_get_buffered_image, "getbufferedimage");
