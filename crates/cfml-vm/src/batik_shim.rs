//! `org.apache.batik.transcoder.*` — Batik's SVG transcoder, over the
//! `imageReadSvg()` and `imageWrite()` builtins.
//!
//! Preside's `SvgToPngService` is the caller: an uploaded SVG asset is
//! rasterised to PNG so the rest of the asset pipeline (thumbnails, derivatives,
//! image sizing) has something to work with.
//!
//! ```cfml
//! t       = createObject( "java", "org.apache.batik.transcoder.image.PNGTranscoder", lib ).init();
//! svgURI  = createObject( "java", "java.io.File" ).init( svgFilePath ).toURL().toString();
//! input   = createObject( "java", "org.apache.batik.transcoder.TranscoderInput", lib ).init( svgURI );
//! ostream = createObject( "java", "java.io.FileOutputStream" ).init( pngFilePath );
//! output  = createObject( "java", "org.apache.batik.transcoder.TranscoderOutput", lib ).init( ostream );
//! t.addTranscodingHint( t.KEY_WIDTH, JavaCast( "float", width ) );
//! t.transcode( input, output );
//! ```
//!
//! Batik is a *transcoder*: hints go in, then one `transcode()` call does the
//! work. The shim keeps that shape — hints accumulate on the transcoder handle
//! and nothing happens until `transcode()`, which reads the input, rasterises
//! through `imageReadSvg()` and writes through `imageWrite()`.
//!
//! **`KEY_WIDTH`/`KEY_HEIGHT` are read as FIELDS** (`t.KEY_WIDTH`), not getters,
//! so they are real keys on the transcoder struct — carrying the hint name the
//! shim then looks for in `addTranscodingHint`.
//!
//! The input arrives as a `file:` URI, because the caller built it from
//! `java.io.File.toURL()`. `TranscoderOutput` wraps a
//! `java.io.FileOutputStream`, which already knows its own path — so both ends
//! of the transcode resolve to plain filesystem paths.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

pub const PNG_TRANSCODER: &str = "org.apache.batik.transcoder.image.pngtranscoder";
pub const JPEG_TRANSCODER: &str = "org.apache.batik.transcoder.image.jpegtranscoder";
pub const TIFF_TRANSCODER: &str = "org.apache.batik.transcoder.image.tifftranscoder";
pub const TRANSCODER_INPUT: &str = "org.apache.batik.transcoder.transcoderinput";
pub const TRANSCODER_OUTPUT: &str = "org.apache.batik.transcoder.transcoderoutput";

pub fn is_batik_class(class_lower: &str) -> bool {
    matches!(
        class_lower,
        PNG_TRANSCODER | JPEG_TRANSCODER | TIFF_TRANSCODER | TRANSCODER_INPUT | TRANSCODER_OUTPUT
    )
}

fn is_transcoder(class_lower: &str) -> bool {
    matches!(class_lower, PNG_TRANSCODER | JPEG_TRANSCODER | TIFF_TRANSCODER)
}

/// The image format a transcoder class produces.
fn output_format(class_lower: &str) -> &'static str {
    match class_lower {
        JPEG_TRANSCODER => "jpg",
        TIFF_TRANSCODER => "tiff",
        _ => "png",
    }
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

pub fn construct(class_lower: &str) -> CfmlResult {
    let mut m = shim(class_lower);
    if is_transcoder(class_lower) {
        // Batik's transcoding-hint keys are public static fields, read as
        // `t.KEY_WIDTH`. The value is the hint name addTranscodingHint() then
        // matches on, so the round trip needs no separate mapping.
        for key in [
            "KEY_WIDTH",
            "KEY_HEIGHT",
            "KEY_MAX_WIDTH",
            "KEY_MAX_HEIGHT",
            "KEY_BACKGROUND_COLOR",
            "KEY_QUALITY",
        ] {
            m.insert(key.to_string(), CfmlValue::string(key.to_string()));
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

fn transcoder_exception(message: impl std::fmt::Display) -> CfmlError {
    CfmlError::new(
        format!("org.apache.batik.transcoder.TranscoderException: {}", message),
        CfmlErrorType::Custom("org.apache.batik.transcoder.TranscoderException".to_string()),
    )
}

fn unsupported(class: &str, method: &str) -> CfmlError {
    CfmlError::new(
        format!(
            "{}.{}() is not supported by RustCFML's Batik adapter, which covers \
             PNG/JPEG/TIFF transcoding of an SVG file to an image file \
             (TranscoderInput → addTranscodingHint → transcode → TranscoderOutput).",
            class, method
        ),
        CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
    )
}

/// A `file:` URI, a `file:///` URI or a bare path, reduced to a filesystem path.
///
/// `java.io.File.toURL()` emits the single-slash `file:/a/b` form, which is not
/// a valid RFC-8089 URL but is exactly what the JDK has always produced and what
/// callers therefore pass in.
fn path_from_uri(raw: &str) -> String {
    let s = raw.trim();
    let stripped = s
        .strip_prefix("file://localhost")
        .or_else(|| s.strip_prefix("file://"))
        .or_else(|| s.strip_prefix("file:"))
        .unwrap_or(s);
    // Percent-decoding, for a path that contained spaces or non-ASCII.
    let bytes = stripped.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `read_svg` is `imageReadSvg()`; `write_image` is `imageWrite()`.
pub fn dispatch(
    class_lower: &str,
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    read_svg: &dyn Fn(Vec<CfmlValue>) -> CfmlResult,
    write_image: &dyn Fn(Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    match class_lower {
        TRANSCODER_INPUT => match method {
            // TranscoderInput( uri ) — also accepts a Reader/InputStream in
            // Batik; here it is the URI form callers build from File.toURL().
            "init" => {
                let mut m = shim(TRANSCODER_INPUT);
                let arg = args.first().cloned().unwrap_or(CfmlValue::Null);
                // A java.io.File / FileInputStream shim knows its own path;
                // otherwise it is the URI string.
                let path = get(&arg, "__file_path")
                    .or_else(|| get(&arg, "__stream_path"))
                    .map(|v| v.as_string())
                    .unwrap_or_else(|| path_from_uri(&arg.as_string()));
                m.insert("__path".to_string(), CfmlValue::string(path));
                Ok(CfmlValue::strukt(m))
            }
            "geturi" | "getinputstream" => {
                Ok(get(object, "__path").unwrap_or(CfmlValue::Null))
            }
            other => Err(unsupported("org.apache.batik.transcoder.TranscoderInput", other)),
        },

        TRANSCODER_OUTPUT => match method {
            // TranscoderOutput( OutputStream ) — the stream already knows where
            // it points, which is all the transcode needs.
            "init" => {
                let mut m = shim(TRANSCODER_OUTPUT);
                let arg = args.first().cloned().unwrap_or(CfmlValue::Null);
                let path = get(&arg, "__stream_path")
                    .or_else(|| get(&arg, "__path"))
                    .or_else(|| get(&arg, "__file_path"))
                    .map(|v| v.as_string())
                    .unwrap_or_else(|| path_from_uri(&arg.as_string()));
                m.insert("__path".to_string(), CfmlValue::string(path));
                Ok(CfmlValue::strukt(m))
            }
            "getoutputstream" | "geturi" => {
                Ok(get(object, "__path").unwrap_or(CfmlValue::Null))
            }
            other => Err(unsupported("org.apache.batik.transcoder.TranscoderOutput", other)),
        },

        _ => match method {
            "init" => construct(class_lower),
            // Hints accumulate; nothing is computed until transcode().
            "addtranscodinghint" => {
                let key = args.first().map(|v| v.as_string()).unwrap_or_default();
                let value = args.get(1).cloned().unwrap_or(CfmlValue::Null);
                if let CfmlValue::Struct(s) = object {
                    s.insert(format!("__hint_{}", key.to_ascii_lowercase()), value);
                }
                Ok(CfmlValue::Null)
            }
            "settranscodinghints" => Ok(CfmlValue::Null),
            "transcode" => {
                let input = args.first().cloned().unwrap_or(CfmlValue::Null);
                let output = args.get(1).cloned().unwrap_or(CfmlValue::Null);
                let src = get(&input, "__path").map(|v| v.as_string()).unwrap_or_default();
                let dest = get(&output, "__path").map(|v| v.as_string()).unwrap_or_default();
                if src.is_empty() {
                    return Err(transcoder_exception(
                        "the TranscoderInput names no readable SVG file",
                    ));
                }
                if dest.is_empty() {
                    return Err(transcoder_exception(
                        "the TranscoderOutput is not backed by a file — RustCFML's adapter \
                         transcodes file to file, so give it a java.io.FileOutputStream",
                    ));
                }

                let hint = |name: &str| -> CfmlValue {
                    get(object, &format!("__hint_{}", name)).unwrap_or(CfmlValue::Null)
                };
                // KEY_MAX_* are a ceiling rather than a target; with no explicit
                // KEY_WIDTH/HEIGHT they are the closest thing to one, and
                // imageReadSvg fits inside a box, so they map cleanly.
                let width = match hint("key_width") {
                    CfmlValue::Null => hint("key_max_width"),
                    v => v,
                };
                let height = match hint("key_height") {
                    CfmlValue::Null => hint("key_max_height"),
                    v => v,
                };

                let img = read_svg(vec![CfmlValue::string(src.clone()), width, height])
                    .map_err(|e| transcoder_exception(e.message))?;
                write_image(vec![
                    img,
                    CfmlValue::string(dest),
                    // Overwrite: the caller made the destination file itself,
                    // by constructing the FileOutputStream.
                    CfmlValue::Null,
                    CfmlValue::Bool(true),
                ])
                .map_err(|e| transcoder_exception(e.message))?;
                Ok(CfmlValue::Null)
            }
            other => Err(unsupported(
                match class_lower {
                    JPEG_TRANSCODER => "org.apache.batik.transcoder.image.JPEGTranscoder",
                    TIFF_TRANSCODER => "org.apache.batik.transcoder.image.TIFFTranscoder",
                    _ => "org.apache.batik.transcoder.image.PNGTranscoder",
                },
                other,
            )),
        },
    }
}
