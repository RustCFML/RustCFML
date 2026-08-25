//! `net.glxn.qrgen.*` — the QRGen fluent builder, over the `qrCodeGenerate()`
//! builtin.
//!
//! Preside's `QrCodeGenerator.cfc` is the caller, and `LoginService` uses it to
//! render the enrolment code an authenticator app scans during two-factor setup
//! — so it pairs directly with the TOTP surface in `javax_crypto_shim`.
//!
//! ```cfml
//! qrCode = CreateObject( "java", "net.glxn.qrgen.javase.QRCode", jars );
//! binary = qrCode.from( input )
//!                .to( imageTypes.GIF )
//!                .withSize( size, size )
//!                .stream()
//!                .toByteArray();
//! ```
//!
//! QRGen is a **builder**: `from()` starts one and every subsequent call
//! refines it, with nothing computed until `stream()`/`file()`. The shim keeps
//! the same shape — each step returns a new handle carrying the accumulated
//! settings, and the QR code is generated exactly once, at `stream()`.
//!
//! `stream()` hands back an already-populated stream whose `toByteArray()`
//! yields a **Binary**. QRGen's own return type is a
//! `java.io.ByteArrayOutputStream`, but that shim answers with the signed-byte
//! ARRAY form — correct for `String.getBytes()`, and what the TOTP path in
//! `javax_crypto_shim` depends on. Image bytes are binary, and the caller's
//! function is declared `binary function`, so this stream is its own type
//! rather than one of those two answers being made wrong.
//!
//! `ImageType` is a Java enum whose members are read as **fields**
//! (`imageTypes.GIF`), so they are real keys on the shim struct, carrying the
//! format name the builtin takes.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

pub const QRCODE_CLASS: &str = "net.glxn.qrgen.javase.qrcode";
pub const IMAGE_TYPE_CLASS: &str = "net.glxn.qrgen.core.image.imagetype";
/// The stream `stream()` hands back. Not a real QRGen type — QRGen returns a
/// `java.io.ByteArrayOutputStream` — but that shim's `toByteArray()` yields the
/// signed-byte ARRAY form, which is right for `String.getBytes()` and which the
/// TOTP path depends on, and wrong here: the caller's function is declared
/// `binary function`, and image bytes are binary. A dedicated stream keeps both
/// answers correct instead of making one of them wrong.
pub const STREAM_CLASS: &str = "net.glxn.qrgen.__bytestream";

pub fn is_qrgen_class(class_lower: &str) -> bool {
    matches!(class_lower, QRCODE_CLASS | IMAGE_TYPE_CLASS | STREAM_CLASS)
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

pub fn construct(class_lower: &str) -> CfmlResult {
    let mut m = shim(class_lower);
    if class_lower == IMAGE_TYPE_CLASS {
        // Enum members, read as fields. The value is the format name
        // `qrCodeGenerate()` takes, so `to( imageTypes.GIF )` needs no mapping.
        for name in ["GIF", "PNG", "JPG", "JPEG", "BMP"] {
            m.insert(name.to_string(), CfmlValue::string(name.to_ascii_lowercase()));
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

fn unsupported(method: &str) -> CfmlError {
    CfmlError::new(
        format!(
            "net.glxn.qrgen.javase.QRCode.{}() is not supported by RustCFML's QRGen adapter, \
             which covers from() → to() / withSize() / withCharset() / withErrorCorrection() \
             → stream(). Anything beyond that is refused rather than silently ignored, since \
             a QR code that encodes the wrong thing still scans.",
            method
        ),
        CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
    )
}

/// Carry the builder forward with one setting changed.
fn refine(object: &CfmlValue, updates: &[(&str, CfmlValue)]) -> CfmlValue {
    let mut m = match object {
        CfmlValue::Struct(s) => s.snapshot(),
        _ => shim(QRCODE_CLASS),
    };
    for (k, v) in updates {
        m.insert(k.to_string(), v.clone());
    }
    CfmlValue::strukt(m)
}

/// `generate` is the `qrCodeGenerate()` builtin.
pub fn dispatch(
    class_lower: &str,
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    generate: &dyn Fn(Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    if class_lower == STREAM_CLASS {
        return match method {
            "tobytearray" => Ok(get(object, "__bytes").unwrap_or(CfmlValue::Binary(Vec::new()))),
            "size" => Ok(CfmlValue::Int(match get(object, "__bytes") {
                Some(CfmlValue::Binary(b)) => b.len() as i64,
                _ => 0,
            })),
            "close" | "flush" | "reset" => Ok(CfmlValue::Null),
            other => Err(CfmlError::new(
                format!(
                    "The stream returned by QRGen's stream() supports toByteArray()/size()/\
                     close(); {}() is not supported.",
                    other
                ),
                CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
            )),
        };
    }
    if class_lower == IMAGE_TYPE_CLASS {
        // The enum's members are fields, handled at construction; a method call
        // on it is not something QRGen callers make.
        return Err(CfmlError::new(
            format!(
                "net.glxn.qrgen.core.image.ImageType.{}() is not supported — ImageType is an \
                 enum, read as a field (imageTypes.GIF).",
                method
            ),
            CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
        ));
    }

    match method {
        "init" => Ok(CfmlValue::strukt(shim(QRCODE_CLASS))),
        // Static: starts a builder holding the text to encode.
        "from" => Ok(refine(
            &CfmlValue::strukt(shim(QRCODE_CLASS)),
            &[(
                "__text",
                CfmlValue::string(args.first().map(|v| v.as_string()).unwrap_or_default()),
            )],
        )),
        "to" => Ok(refine(
            object,
            &[(
                "__format",
                CfmlValue::string(args.first().map(|v| v.as_string()).unwrap_or_default()),
            )],
        )),
        // withSize( width, height ). A QR symbol is square, so a non-square
        // request cannot be honoured as asked; the SMALLER edge is used, which
        // keeps the code inside the box the caller reserved for it.
        "withsize" => {
            let n = |i: usize| -> i64 {
                args.get(i)
                    .map(|v| v.as_string().trim().parse().unwrap_or(0))
                    .unwrap_or(0)
            };
            let (w, h) = (n(0), n(1));
            let edge = if h > 0 { w.min(h) } else { w };
            Ok(refine(object, &[("__size", CfmlValue::Int(edge))]))
        }
        "witherrorcorrection" => Ok(refine(
            object,
            &[(
                "__ec",
                CfmlValue::string(args.first().map(|v| v.as_string()).unwrap_or_default()),
            )],
        )),
        // Charset only affects how the text is turned into bytes; the builtin
        // encodes UTF-8, which is what QRGen defaults to and what scanners
        // expect. Accepted and ignored (docs/known-issues.md).
        "withcharset" | "withhint" | "withhints" => Ok(object.clone()),
        // The one call that actually generates. Returns a populated
        // ByteArrayOutputStream, because `.toByteArray()` is what comes next.
        "stream" | "file" => {
            let text = get(object, "__text").map(|v| v.as_string()).unwrap_or_default();
            if text.is_empty() {
                return Err(CfmlError::new(
                    "net.glxn.qrgen: nothing to encode — call from( text ) first".to_string(),
                    CfmlErrorType::Custom("java.lang.IllegalStateException".to_string()),
                ));
            }
            let format = get(object, "__format")
                .map(|v| v.as_string())
                .filter(|f| !f.is_empty())
                // QRGen's own default.
                .unwrap_or_else(|| "png".to_string());
            let size = get(object, "__size").unwrap_or(CfmlValue::Int(125));
            let ec = get(object, "__ec").unwrap_or(CfmlValue::Null);

            let binary = generate(vec![
                CfmlValue::string(text),
                size,
                CfmlValue::string(format),
                ec,
            ])?;

            if method == "file" {
                return Err(CfmlError::new(
                    "net.glxn.qrgen.javase.QRCode.file() is not supported: it writes to a JVM \
                     temp File. Use .stream().toByteArray() and fileWrite() the result."
                        .to_string(),
                    CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
                ));
            }

            let mut stream = shim(STREAM_CLASS);
            stream.insert(
                "__bytes".to_string(),
                match binary {
                    CfmlValue::Binary(b) => CfmlValue::Binary(b),
                    other => CfmlValue::Binary(other.as_string().into_bytes()),
                },
            );
            Ok(CfmlValue::strukt(stream))
        }
        other => Err(unsupported(other)),
    }
}
