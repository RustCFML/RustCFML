//! `javax.crypto.*` and `java.security.SecureRandom` — the JCE surface CFML code
//! reaches for when the language has no BIF for what it needs.
//!
//! The motivating caller is Preside's `GoogleAuthenticator.cfc` (TOTP two-factor
//! auth), which is MIT-licensed code vendored into Preside and used verbatim by
//! several other CFML projects. It needs three things the CFML standard library
//! does not expose:
//!
//! ```cfml
//! secretKeySpec = CreateObject( "java", "javax.crypto.spec.SecretKeySpec" ).init( key, "HmacSHA1" );
//! mac           = CreateObject( "java", "javax.crypto.Mac" ).getInstance( secretKeySpec.getAlgorithm() );
//! mac.init( secretKeySpec );
//! h             = mac.doFinal( buffer.array() );          // raw byte[] HMAC
//!
//! secureRandom  = CreateObject( "java", "java.security.SecureRandom" ).init();
//! secureRandom.nextBytes( salt );                         // fills byte[16] IN PLACE
//!
//! keyFactory    = CreateObject( "java", "javax.crypto.SecretKeyFactory" ).getInstance( "PBKDF2WithHmacSHA1" );
//! keySpec       = CreateObject( "java", "javax.crypto.spec.PBEKeySpec" ).init( pw.toCharArray(), salt, 128, 80 );
//! secretKey     = keyFactory.generateSecret( keySpec ).getEncoded();
//! ```
//!
//! Each maps onto a builtin, which is where the actual cryptography lives:
//! `Mac` → `hmac()`, `SecretKeyFactory`/`PBEKeySpec` → `generatePBKDFKey()`,
//! `SecureRandom` → `randomBytes()`. Two of those BIFs were text-only before this
//! shim existed (they ran their inputs through `as_string()`, which mangles raw
//! bytes); they now take bytes verbatim, which is a fix for direct CFML callers
//! too, not just for this adapter.
//!
//! **Byte arrays.** Java hands CFML a `byte[]` as a 1-based array of *signed*
//! bytes, and the calling code depends on that representation — `GoogleAuthenticator`
//! does `t = h[20]; if (t < 0) t += 256;` on every byte. So `doFinal()` and
//! `getEncoded()` return `Array(Int)` in `-128..=127`, matching what the
//! `ByteArrayOutputStream`/`ByteBuffer` shims already produce, and every method
//! here accepts either that form or a `Binary`.
//!
//! **`nextBytes` mutates its argument.** Java's `SecureRandom.nextBytes(byte[])`
//! fills the caller's array; `generateKey()` above relies on it — the salt it
//! passes in is the salt it then derives from. RustCFML arrays are
//! `Arc<RwLock<Vec>>` handles, so the shim writes through the handle it was given
//! and the caller sees the bytes. Returning a fresh array instead would leave the
//! caller deriving every key from an all-zero salt, which is precisely the kind of
//! silent, security-relevant wrong answer a shim must not produce.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};

pub const MAC_CLASS: &str = "javax.crypto.mac";
pub const SECRET_KEY_SPEC_CLASS: &str = "javax.crypto.spec.secretkeyspec";
pub const SECRET_KEY_FACTORY_CLASS: &str = "javax.crypto.secretkeyfactory";
pub const PBE_KEY_SPEC_CLASS: &str = "javax.crypto.spec.pbekeyspec";
pub const SECRET_KEY_CLASS: &str = "javax.crypto.secretkey";
pub const SECURE_RANDOM_CLASS: &str = "java.security.securerandom";

pub fn is_crypto_class(class_lower: &str) -> bool {
    matches!(
        class_lower,
        MAC_CLASS
            | SECRET_KEY_SPEC_CLASS
            | SECRET_KEY_FACTORY_CLASS
            | PBE_KEY_SPEC_CLASS
            | SECRET_KEY_CLASS
            | SECURE_RANDOM_CLASS
    )
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m
}

pub fn construct(class_lower: &str) -> CfmlResult {
    Ok(CfmlValue::strukt(shim(class_lower)))
}

fn field(object: &CfmlValue, key: &str) -> Option<CfmlValue> {
    match object {
        CfmlValue::Struct(s) => s.get(key),
        _ => None,
    }
}

fn field_str(object: &CfmlValue, key: &str) -> String {
    field(object, key).map(|v| v.as_string()).unwrap_or_default()
}

fn field_int(object: &CfmlValue, key: &str, default: i64) -> i64 {
    match field(object, key) {
        Some(CfmlValue::Int(n)) => n,
        Some(CfmlValue::Double(d)) => d as i64,
        Some(other) => other.as_string().trim().parse().unwrap_or(default),
        None => default,
    }
}

/// A CFML value used as a Java `byte[]`: `Binary` verbatim, an array of signed
/// byte ints masked to 8 bits, anything else via its UTF-8 form. A `char[]` (what
/// `String.toCharArray()` yields) also lands here and is taken as its characters'
/// bytes, which is what `PBEKeySpec(char[], …)` means.
///
/// A `Key` object — `SecretKeySpec` or the `SecretKey` a factory produced — yields
/// its key material. `Mac.init()` takes a `Key`, not a `byte[]`, and without this
/// the receiver would be hashed as the *text* of a struct: the HMAC still computed,
/// still looked like a token, and was wrong. That failure is invisible to any test
/// that only round-trips generate-then-verify against itself, which is exactly how
/// a TOTP implementation gets shipped broken — it takes an external vector
/// (RFC 4226) to catch it.
fn to_bytes(v: &CfmlValue) -> Vec<u8> {
    if let CfmlValue::Struct(s) = v {
        if s.contains_key("__java_shim") {
            if let Some(CfmlValue::Binary(b)) = s.get("__key") {
                return b;
            }
        }
    }
    match v {
        CfmlValue::Binary(b) => b.clone(),
        CfmlValue::Array(a) => {
            let items = a.snapshot();
            // A char[] arrives as one-character strings; a byte[] as ints.
            if items.iter().any(|e| matches!(e, CfmlValue::String(_))) {
                items.iter().flat_map(|e| e.as_string().into_bytes()).collect()
            } else {
                items
                    .iter()
                    .map(|e| match e {
                        CfmlValue::Int(i) => (*i & 0xFF) as u8,
                        CfmlValue::Double(d) => (*d as i64 & 0xFF) as u8,
                        other => other.as_string().trim().parse::<i64>().unwrap_or(0) as u8,
                    })
                    .collect()
            }
        }
        other => other.as_string().into_bytes(),
    }
}

/// Bytes back out in the shape Java gives CFML: a 1-based array of signed bytes.
fn signed_array(bytes: &[u8]) -> CfmlValue {
    CfmlValue::array(bytes.iter().map(|b| CfmlValue::Int(*b as i8 as i64)).collect())
}

fn decode_hex(hex: &str) -> Vec<u8> {
    let raw: Vec<u8> = hex.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    raw.chunks(2)
        .filter(|c| c.len() == 2)
        .filter_map(|c| u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok())
        .collect()
}

fn decode_base64(s: &str) -> Vec<u8> {
    // Minimal, dependency-free standard-alphabet decoder. `generatePBKDFKey`
    // returns base64 and this is the only place it needs undoing.
    const TAB: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::new();
    for ch in s.bytes() {
        if ch == b'=' || ch.is_ascii_whitespace() {
            continue;
        }
        let Some(idx) = TAB.iter().position(|c| *c == ch) else {
            continue;
        };
        acc = (acc << 6) | idx as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

fn no_such_algorithm(alg: &str) -> CfmlError {
    CfmlError::new(
        format!("java.security.NoSuchAlgorithmException: {}", alg),
        CfmlErrorType::Custom("java.security.NoSuchAlgorithmException".to_string()),
    )
}

fn unsupported(class: &str, method: &str) -> CfmlError {
    CfmlError::new(
        format!(
            "{}.{}() is not supported by RustCFML's JCE shim",
            class, method
        ),
        CfmlErrorType::Custom("java.lang.UnsupportedOperationException".to_string()),
    )
}

/// Map a JCE `Mac` algorithm name onto the `hmac()` builtin's spelling.
/// Returns `None` for anything the builtin cannot do, so the caller raises
/// `NoSuchAlgorithmException` rather than hashing with the wrong function.
fn mac_algorithm(name: &str) -> Option<&'static str> {
    match name.to_ascii_uppercase().replace(['-', '_'], "").as_str() {
        "HMACMD5" => Some("HMACMD5"),
        "HMACSHA1" => Some("HMACSHA1"),
        "HMACSHA256" => Some("HMACSHA256"),
        "HMACSHA384" => Some("HMACSHA384"),
        "HMACSHA512" => Some("HMACSHA512"),
        _ => None,
    }
}

// ── javax.crypto.spec.SecretKeySpec ──────────────────────────────────────────

pub fn handle_secret_key_spec(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        // SecretKeySpec(byte[] key, String algorithm) — and the (key, off, len,
        // algorithm) overload, whose algorithm is the LAST argument.
        "init" => {
            let key = args.first().map(to_bytes).unwrap_or_default();
            let (key, algorithm) = if args.len() >= 4 {
                let off = args[1].as_string().trim().parse::<usize>().unwrap_or(0);
                let len = args[2].as_string().trim().parse::<usize>().unwrap_or(0);
                let end = (off + len).min(key.len());
                (
                    if off <= end { key[off..end].to_vec() } else { Vec::new() },
                    args[3].as_string(),
                )
            } else {
                (key, args.get(1).map(|v| v.as_string()).unwrap_or_default())
            };
            let mut m = shim(SECRET_KEY_SPEC_CLASS);
            m.insert("__key".to_string(), CfmlValue::Binary(key));
            m.insert("__algorithm".to_string(), CfmlValue::string(algorithm));
            Ok(CfmlValue::strukt(m))
        }
        "getalgorithm" => Ok(CfmlValue::string(field_str(object, "__algorithm"))),
        "getencoded" => Ok(signed_array(&match field(object, "__key") {
            Some(CfmlValue::Binary(b)) => b,
            _ => Vec::new(),
        })),
        "getformat" => Ok(CfmlValue::string("RAW".to_string())),
        other => Err(unsupported("javax.crypto.spec.SecretKeySpec", other)),
    }
}

// ── javax.crypto.Mac ─────────────────────────────────────────────────────────

/// `hmac` is the `hmac()` builtin, injected by the caller.
pub fn handle_mac(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    hmac: impl FnOnce(Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    match method {
        // Static factory, called on the un-init'ed class object.
        "getinstance" => {
            let requested = args.first().map(|v| v.as_string()).unwrap_or_default();
            let alg = mac_algorithm(&requested).ok_or_else(|| no_such_algorithm(&requested))?;
            let mut m = shim(MAC_CLASS);
            m.insert("__algorithm".to_string(), CfmlValue::string(alg.to_string()));
            // Java's requested spelling, for getAlgorithm() round-tripping.
            m.insert("__requested".to_string(), CfmlValue::string(requested));
            Ok(CfmlValue::strukt(m))
        }
        // init(Key) — stash the key material on the receiver, in place, so the
        // handle the caller holds is the initialised one.
        "init" => {
            let key = args.first().map(to_bytes).unwrap_or_default();
            if let CfmlValue::Struct(s) = object {
                s.insert("__key".to_string(), CfmlValue::Binary(key));
            }
            Ok(CfmlValue::Null)
        }
        "reset" => {
            if let CfmlValue::Struct(s) = object {
                s.insert("__pending".to_string(), CfmlValue::Binary(Vec::new()));
            }
            Ok(CfmlValue::Null)
        }
        // update(byte[]) accumulates; doFinal() hashes what accumulated, and
        // doFinal(byte[]) hashes the accumulation plus its argument (Java's
        // contract), then resets.
        "update" => {
            let mut pending = match field(object, "__pending") {
                Some(CfmlValue::Binary(b)) => b,
                _ => Vec::new(),
            };
            if let Some(a) = args.first() {
                pending.extend_from_slice(&to_bytes(a));
            }
            if let CfmlValue::Struct(s) = object {
                s.insert("__pending".to_string(), CfmlValue::Binary(pending));
            }
            Ok(CfmlValue::Null)
        }
        "dofinal" => {
            let key = match field(object, "__key") {
                Some(CfmlValue::Binary(b)) => b,
                _ => {
                    return Err(CfmlError::new(
                        "java.lang.IllegalStateException: MAC not initialized — call init(key) \
                         before doFinal()"
                            .to_string(),
                        CfmlErrorType::Custom("java.lang.IllegalStateException".to_string()),
                    ))
                }
            };
            let mut message = match field(object, "__pending") {
                Some(CfmlValue::Binary(b)) => b,
                _ => Vec::new(),
            };
            if let Some(a) = args.first() {
                message.extend_from_slice(&to_bytes(a));
            }
            let alg = field_str(object, "__algorithm");

            let hex = hmac(vec![
                CfmlValue::Binary(message),
                CfmlValue::Binary(key),
                CfmlValue::string(alg),
            ])?
            .as_string();

            // Java resets the Mac after doFinal, so a second message does not
            // silently inherit the first one's bytes.
            if let CfmlValue::Struct(s) = object {
                s.insert("__pending".to_string(), CfmlValue::Binary(Vec::new()));
            }
            Ok(signed_array(&decode_hex(&hex)))
        }
        "getalgorithm" => Ok(CfmlValue::string(field_str(object, "__requested"))),
        "getmaclength" => Ok(CfmlValue::Int(match field_str(object, "__algorithm").as_str() {
            "HMACMD5" => 16,
            "HMACSHA1" => 20,
            "HMACSHA256" => 32,
            "HMACSHA384" => 48,
            "HMACSHA512" => 64,
            _ => 0,
        })),
        other => Err(unsupported("javax.crypto.Mac", other)),
    }
}

// ── javax.crypto.spec.PBEKeySpec ─────────────────────────────────────────────

pub fn handle_pbe_key_spec(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        // PBEKeySpec(char[] password, byte[] salt, int iterationCount, int keyLength)
        "init" => {
            let mut m = shim(PBE_KEY_SPEC_CLASS);
            m.insert(
                "__password".to_string(),
                CfmlValue::Binary(args.first().map(to_bytes).unwrap_or_default()),
            );
            m.insert(
                "__salt".to_string(),
                CfmlValue::Binary(args.get(1).map(to_bytes).unwrap_or_default()),
            );
            let num = |i: usize| -> i64 {
                args.get(i)
                    .map(|v| v.as_string().trim().parse::<i64>().unwrap_or(0))
                    .unwrap_or(0)
            };
            m.insert("__iterations".to_string(), CfmlValue::Int(num(2)));
            m.insert("__keylength".to_string(), CfmlValue::Int(num(3)));
            Ok(CfmlValue::strukt(m))
        }
        "getiterationcount" => Ok(CfmlValue::Int(field_int(object, "__iterations", 0))),
        "getkeylength" => Ok(CfmlValue::Int(field_int(object, "__keylength", 0))),
        "getsalt" => Ok(signed_array(&match field(object, "__salt") {
            Some(CfmlValue::Binary(b)) => b,
            _ => Vec::new(),
        })),
        // Java hands back a char[]; a CFML caller almost always just wants it
        // back as the characters it put in.
        "getpassword" => Ok(CfmlValue::array(
            match field(object, "__password") {
                Some(CfmlValue::Binary(b)) => String::from_utf8_lossy(&b).to_string(),
                _ => String::new(),
            }
            .chars()
            .map(|c| CfmlValue::string(c.to_string()))
            .collect(),
        )),
        "clearpassword" => {
            if let CfmlValue::Struct(s) = object {
                s.insert("__password".to_string(), CfmlValue::Binary(Vec::new()));
            }
            Ok(CfmlValue::Null)
        }
        other => Err(unsupported("javax.crypto.spec.PBEKeySpec", other)),
    }
}

// ── javax.crypto.SecretKeyFactory ────────────────────────────────────────────

/// `pbkdf` is the `generatePBKDFKey()` builtin (which returns base64).
pub fn handle_secret_key_factory(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
    pbkdf: impl FnOnce(Vec<CfmlValue>) -> CfmlResult,
) -> CfmlResult {
    match method {
        "getinstance" => {
            let requested = args.first().map(|v| v.as_string()).unwrap_or_default();
            let normalized = requested.to_ascii_uppercase().replace(['-', '_'], "");
            // Only the PBKDF2 family is representable; anything else (DES,
            // PBEWithMD5AndDES, …) would need a different derivation entirely.
            if !normalized.starts_with("PBKDF2") {
                return Err(no_such_algorithm(&requested));
            }
            let mut m = shim(SECRET_KEY_FACTORY_CLASS);
            m.insert("__algorithm".to_string(), CfmlValue::string(requested));
            Ok(CfmlValue::strukt(m))
        }
        "generatesecret" => {
            let spec = args.first().cloned().unwrap_or(CfmlValue::Null);
            let password = match field(&spec, "__password") {
                Some(CfmlValue::Binary(b)) => b,
                _ => {
                    return Err(CfmlError::new(
                        "java.security.spec.InvalidKeySpecException: generateSecret() requires a \
                         javax.crypto.spec.PBEKeySpec"
                            .to_string(),
                        CfmlErrorType::Custom(
                            "java.security.spec.InvalidKeySpecException".to_string(),
                        ),
                    ))
                }
            };
            let salt = match field(&spec, "__salt") {
                Some(CfmlValue::Binary(b)) => b,
                _ => Vec::new(),
            };
            let iterations = field_int(&spec, "__iterations", 0);
            let key_length = field_int(&spec, "__keylength", 0);

            let b64 = pbkdf(vec![
                CfmlValue::string(field_str(object, "__algorithm")),
                CfmlValue::Binary(password),
                CfmlValue::Binary(salt),
                CfmlValue::Int(iterations),
                CfmlValue::Int(key_length),
            ])?
            .as_string();

            let mut m = shim(SECRET_KEY_CLASS);
            m.insert("__key".to_string(), CfmlValue::Binary(decode_base64(&b64)));
            m.insert(
                "__algorithm".to_string(),
                CfmlValue::string(field_str(object, "__algorithm")),
            );
            Ok(CfmlValue::strukt(m))
        }
        "getalgorithm" => Ok(CfmlValue::string(field_str(object, "__algorithm"))),
        other => Err(unsupported("javax.crypto.SecretKeyFactory", other)),
    }
}

/// The `SecretKey` a `SecretKeyFactory` produces. Shares `SecretKeySpec`'s
/// getters — it is the same `javax.crypto.SecretKey` interface.
pub fn handle_secret_key(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    handle_secret_key_spec(method, args, object)
}

// ── java.security.SecureRandom ───────────────────────────────────────────────

/// `random_bytes` is the `randomBytes()` builtin.
pub fn handle_secure_random(
    method: &str,
    args: Vec<CfmlValue>,
    random_bytes: impl Fn(i64) -> CfmlResult,
) -> CfmlResult {
    match method {
        // init() / init(seed) / getInstance(alg): all yield the same OS CSPRNG.
        // A supplied seed is deliberately IGNORED rather than honoured — Java's
        // SecureRandom(byte[] seed) still mixes in system entropy, so honouring
        // it as a deterministic seed would make output *less* random than the
        // caller's Java behaviour, and reproducing Java's exact mixing is not
        // possible. Ignoring it keeps every draw cryptographically strong.
        "init" | "getinstance" | "setseed" | "generateseed" | "reseed" => {
            if method == "generateseed" {
                let n = args
                    .first()
                    .map(|v| v.as_string().trim().parse::<i64>().unwrap_or(0))
                    .unwrap_or(0);
                let bytes = random_bytes(n.max(0))?;
                return Ok(signed_array(&match bytes {
                    CfmlValue::Binary(b) => b,
                    other => to_bytes(&other),
                }));
            }
            Ok(CfmlValue::strukt(shim(SECURE_RANDOM_CLASS)))
        }
        // nextBytes(byte[]) fills the CALLER'S array in place — see the module
        // note. Anything other than an array to write into is an error, not a
        // silently discarded draw.
        "nextbytes" => {
            let target = args.first().cloned().unwrap_or(CfmlValue::Null);
            let CfmlValue::Array(arr) = target else {
                return Err(CfmlError::new(
                    "java.lang.IllegalArgumentException: SecureRandom.nextBytes() requires a \
                     byte array to fill"
                        .to_string(),
                    CfmlErrorType::Custom("java.lang.IllegalArgumentException".to_string()),
                ));
            };
            let len = arr.len();
            if len == 0 {
                return Ok(CfmlValue::Null);
            }
            let bytes = match random_bytes(len as i64)? {
                CfmlValue::Binary(b) => b,
                other => to_bytes(&other),
            };
            for (i, b) in bytes.iter().enumerate().take(len) {
                arr.set(i, CfmlValue::Int(*b as i8 as i64));
            }
            Ok(CfmlValue::Null)
        }
        "nextint" => {
            let bytes = match random_bytes(4)? {
                CfmlValue::Binary(b) => b,
                other => to_bytes(&other),
            };
            let raw = i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            match args.first() {
                // nextInt(bound): uniform in [0, bound).
                Some(b) => {
                    let bound = b.as_string().trim().parse::<i64>().unwrap_or(0);
                    if bound <= 0 {
                        return Err(CfmlError::new(
                            "java.lang.IllegalArgumentException: bound must be positive"
                                .to_string(),
                            CfmlErrorType::Custom(
                                "java.lang.IllegalArgumentException".to_string(),
                            ),
                        ));
                    }
                    Ok(CfmlValue::Int((raw as i64).rem_euclid(bound)))
                }
                None => Ok(CfmlValue::Int(raw as i64)),
            }
        }
        "nextlong" => {
            let bytes = match random_bytes(8)? {
                CfmlValue::Binary(b) => b,
                other => to_bytes(&other),
            };
            let mut a = [0u8; 8];
            a.copy_from_slice(&bytes[..8]);
            Ok(CfmlValue::Int(i64::from_be_bytes(a)))
        }
        "nextboolean" => {
            let bytes = match random_bytes(1)? {
                CfmlValue::Binary(b) => b,
                other => to_bytes(&other),
            };
            Ok(CfmlValue::Bool(bytes[0] & 1 == 1))
        }
        "nextdouble" | "nextfloat" => {
            let bytes = match random_bytes(8)? {
                CfmlValue::Binary(b) => b,
                other => to_bytes(&other),
            };
            let mut a = [0u8; 8];
            a.copy_from_slice(&bytes[..8]);
            // 53 bits of mantissa, the same construction java.util.Random uses.
            let v = (u64::from_be_bytes(a) >> 11) as f64 / (1u64 << 53) as f64;
            Ok(CfmlValue::Double(v))
        }
        "getalgorithm" => Ok(CfmlValue::string("NativePRNG".to_string())),
        other => Err(unsupported("java.security.SecureRandom", other)),
    }
}
