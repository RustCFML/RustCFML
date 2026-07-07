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
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;
use std::sync::{Arc, RwLock};

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
}

impl CfmlImage {
    fn new(img: DynamicImage, source: String, format: ImageFormat) -> Self {
        CfmlImage { img, source, format }
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
    let format = image::guess_format(bytes)
        .map_err(|e| CfmlError::runtime(format!("Unsupported or unrecognised image format: {}", e)))?;
    let img = image::load_from_memory_with_format(bytes, format)
        .or_else(|_| image::load_from_memory(bytes))
        .map_err(|e| CfmlError::runtime(format!("Unable to decode image: {}", e)))?;
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
            Ok(CfmlImage::new(img, String::new(), format).into_value())
        }
        CfmlValue::String(s) => {
            let s = s.as_str();
            // data: URI or something that looks like base64 (no path separators)
            if s.starts_with("data:") {
                let (img, format) = decode_bytes(&decode_base64_source(s))?;
                return Ok(CfmlImage::new(img, String::new(), format).into_value());
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
    Ok(CfmlImage::new(img, source, format).into_value())
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
            Ok(CfmlImage::new(img, String::new(), format).into_value())
        }
        Some(CfmlValue::String(s)) => {
            let s = s.as_str();
            if s.starts_with("data:") {
                let (img, format) = decode_bytes(&decode_base64_source(s))?;
                Ok(CfmlImage::new(img, String::new(), format).into_value())
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
    let (img, format) = decode_bytes(&decode_base64_source(&s))?;
    Ok(CfmlImage::new(img, String::new(), format).into_value())
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
                dispatch(&handle, "write", vec![CfmlValue::string(destination), CfmlValue::Bool(true)])?;
            }
            Ok(handle)
        }
        "rotate" => {
            let handle = coerce_to_image(&source)?;
            let angle = get("angle").unwrap_or(CfmlValue::Null);
            dispatch(&handle, "rotate", vec![angle])?;
            if !destination.is_empty() {
                dispatch(&handle, "write", vec![CfmlValue::string(destination), CfmlValue::Bool(true)])?;
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
        "border" | "captcha" => Err(CfmlError::runtime(format!(
            "cfimage action=[{}] is not implemented in this build (Tier 2 image support pending)",
            action
        ))),
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
    /// Only quarter-turn angles (multiples of 90°) are supported without the
    /// drawing tier; arbitrary angles need `imageproc`, which is not built in
    /// this tier. A clear error beats a silently-wrong result.
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
            0 => return Ok(()),
            90 => self.img.rotate90(),
            180 => self.img.rotate180(),
            270 => self.img.rotate270(),
            _ => {
                return Err(CfmlError::runtime(format!(
                    "imageRotate currently supports only multiples of 90 degrees (got {}); arbitrary-angle rotation requires the drawing feature",
                    angle
                )));
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
        info.insert("width".into(), CfmlValue::Int(w as i64));
        info.insert("height".into(), CfmlValue::Int(h as i64));
        info.insert("source".into(), CfmlValue::string(self.source.clone()));
        info.insert("colormodel".into(), color_model_struct(color));
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
    cm.insert("alpha_channel_support".into(), CfmlValue::Bool(has_alpha));
    cm.insert("alpha_premultiplied".into(), CfmlValue::Bool(false));
    cm.insert(
        "transparency".into(),
        CfmlValue::string(if has_alpha { "TRANSLUCENT" } else { "OPAQUE" }),
    );
    cm.insert("pixel_size".into(), CfmlValue::Int(pixel_size as i64));
    cm.insert("num_components".into(), CfmlValue::Int(num_components as i64));
    cm.insert(
        "num_color_components".into(),
        CfmlValue::Int(num_color_components as i64),
    );
    cm.insert(
        "colorspace".into(),
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
    cm.insert("bits_component".into(), CfmlValue::array(bits));
    cm.insert(
        "colormodel_type".into(),
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
    // Named
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
