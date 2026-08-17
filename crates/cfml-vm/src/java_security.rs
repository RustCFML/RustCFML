//! `java.security` asymmetric-crypto shims — `Signature`, `KeyFactory`, the RSA
//! key specs, and `java.math.BigInteger`.
//!
//! This is the surface vendored JWT libraries (jwt-cfml and friends) reach
//! through `createObject("java", …)` to verify and sign RS256 tokens. Scope is
//! deliberately the minimal tier that runs those libraries unmodified: **RSA
//! only, verify/sign only**. EC/ECDSA, `CertificateFactory` and
//! `KeyPairGenerator` are not shimmed — a library asking for them gets the same
//! "class is not supported" error as before, rather than a half-working stub.
//!
//! Two contracts worth keeping in mind when extending this:
//!
//! * **`verify()` returns false, it does not throw.** A bad signature is a
//!   normal answer, and callers map it to their own typed error; throwing would
//!   surface as an engine error instead.
//! * **An unknown algorithm throws `NoSuchAlgorithmException`** rather than
//!   silently substituting another one — the same fail-loudly rule the
//!   `MessageDigest` shim follows.
//!
//! Shim instances mutate **in place** through `CfmlStruct`'s interior
//! mutability (`initVerify`/`initSign`/`update` all return `Null`), so they need
//! none of the receiver write-back machinery. That matters for the streaming
//! shape `sig.update(a); sig.update(b);`, where a copy-on-write shim would
//! silently drop the first chunk unless every call site reassigned.

use cfml_common::dynamic::{CfmlValue, ValueMap};
use cfml_common::vm::{CfmlError, CfmlErrorType, CfmlResult};
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::traits::PublicKeyParts;
use rsa::{BigUint, Pkcs1v15Sign, RsaPrivateKey, RsaPublicKey};
use sha2::Digest;

pub const SIGNATURE_CLASS: &str = "java.security.signature";
pub const KEYFACTORY_CLASS: &str = "java.security.keyfactory";
pub const X509_SPEC_CLASS: &str = "java.security.spec.x509encodedkeyspec";
pub const PKCS8_SPEC_CLASS: &str = "java.security.spec.pkcs8encodedkeyspec";
pub const RSA_PUBLIC_SPEC_CLASS: &str = "java.security.spec.rsapublickeyspec";
pub const PUBLIC_KEY_CLASS: &str = "java.security.publickey";
pub const PRIVATE_KEY_CLASS: &str = "java.security.privatekey";
pub const BIGINTEGER_CLASS: &str = "java.math.biginteger";

/// Every class this module owns, for `createObject("java", …)` dispatch.
pub fn is_java_security_class(class_lower: &str) -> bool {
    matches!(
        class_lower,
        SIGNATURE_CLASS
            | KEYFACTORY_CLASS
            | X509_SPEC_CLASS
            | PKCS8_SPEC_CLASS
            | RSA_PUBLIC_SPEC_CLASS
            | PUBLIC_KEY_CLASS
            | PRIVATE_KEY_CLASS
            | BIGINTEGER_CLASS
    )
}

fn shim(class: &str) -> ValueMap {
    let mut m = ValueMap::default();
    m.insert("__java_class".to_string(), CfmlValue::string(class.to_string()));
    m.insert("__java_shim".to_string(), CfmlValue::Bool(true));
    m
}

fn no_such_algorithm(algorithm: &str) -> CfmlError {
    CfmlError::new(
        format!("{} Signature not available", algorithm),
        CfmlErrorType::Custom("java.security.NoSuchAlgorithmException".to_string()),
    )
}

fn key_error(msg: impl std::fmt::Display) -> CfmlError {
    CfmlError::new(
        format!("java.security.spec.InvalidKeySpecException: {}", msg),
        CfmlErrorType::Custom("java.security.spec.InvalidKeySpecException".to_string()),
    )
}

/// Coerce a CFML value to raw bytes the way the Java shims do elsewhere:
/// `Binary` verbatim, anything else via its UTF-8 string form.
fn bytes_of(v: &CfmlValue) -> Vec<u8> {
    match v {
        CfmlValue::Binary(b) => b.clone(),
        CfmlValue::Array(a) => a
            .snapshot()
            .iter()
            .map(|x| x.as_string().trim().parse::<i64>().unwrap_or(0) as u8)
            .collect(),
        other => other.as_string().into_bytes(),
    }
}

fn field_bytes(object: &CfmlValue, key: &str) -> Vec<u8> {
    match object {
        CfmlValue::Struct(s) => s.get(key).map(|v| bytes_of(&v)).unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn field_string(object: &CfmlValue, key: &str) -> String {
    match object {
        CfmlValue::Struct(s) => s.get(key).map(|v| v.as_string()).unwrap_or_default(),
        _ => String::new(),
    }
}

/// The digest half of a `<digest>with<cipher>` Java signature algorithm name.
/// Only RSA (RSASSA-PKCS1-v1_5) ciphers are supported; anything else is
/// rejected at `getInstance` time.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SigDigest {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

/// Parse e.g. `"SHA256withRSA"` (case-insensitive, Java's own spelling) into
/// its digest. `None` for anything we do not implement — including every non-
/// RSA cipher, so `SHA256withECDSA` fails loudly rather than being verified
/// with the wrong scheme.
fn parse_algorithm(algorithm: &str) -> Option<SigDigest> {
    let normalised = algorithm.to_ascii_uppercase().replace('-', "");
    let (digest, cipher) = normalised.split_once("WITH")?;
    if cipher != "RSA" {
        return None;
    }
    match digest {
        "SHA" | "SHA1" => Some(SigDigest::Sha1),
        "SHA256" => Some(SigDigest::Sha256),
        "SHA384" => Some(SigDigest::Sha384),
        "SHA512" => Some(SigDigest::Sha512),
        _ => None,
    }
}

/// Hash `data` and pair it with the PKCS#1 v1.5 padding scheme carrying the
/// matching digest OID — the two always travel together, and mismatching them
/// produces a signature that verifies nowhere.
fn digest_and_scheme(digest: SigDigest, data: &[u8]) -> (Vec<u8>, Pkcs1v15Sign) {
    match digest {
        SigDigest::Sha1 => (
            sha1::Sha1::digest(data).to_vec(),
            Pkcs1v15Sign::new::<sha1::Sha1>(),
        ),
        SigDigest::Sha256 => (
            sha2::Sha256::digest(data).to_vec(),
            Pkcs1v15Sign::new::<sha2::Sha256>(),
        ),
        SigDigest::Sha384 => (
            sha2::Sha384::digest(data).to_vec(),
            Pkcs1v15Sign::new::<sha2::Sha384>(),
        ),
        SigDigest::Sha512 => (
            sha2::Sha512::digest(data).to_vec(),
            Pkcs1v15Sign::new::<sha2::Sha512>(),
        ),
    }
}

/// Build a `java.security.PublicKey` shim from an RSA modulus/exponent pair.
/// Both arrive as big-endian magnitudes — the JWKS `n`/`e` encoding and what
/// `RsaPublicKey` hands back — so the two construction paths (PEM and JWKS)
/// converge on one representation.
fn public_key_value(n: &[u8], e: &[u8]) -> CfmlValue {
    let mut m = shim(PUBLIC_KEY_CLASS);
    m.insert("__algorithm".to_string(), CfmlValue::string("RSA".to_string()));
    m.insert("__n".to_string(), CfmlValue::Binary(n.to_vec()));
    m.insert("__e".to_string(), CfmlValue::Binary(e.to_vec()));
    CfmlValue::strukt(m)
}

fn rsa_public_from_key_shim(object: &CfmlValue) -> Result<RsaPublicKey, CfmlError> {
    let n = BigUint::from_bytes_be(&field_bytes(object, "__n"));
    let e = BigUint::from_bytes_be(&field_bytes(object, "__e"));
    RsaPublicKey::new(n, e).map_err(key_error)
}

fn rsa_private_from_key_shim(object: &CfmlValue) -> Result<RsaPrivateKey, CfmlError> {
    RsaPrivateKey::from_pkcs8_der(&field_bytes(object, "__pkcs8")).map_err(key_error)
}

/// `java.security.Signature`. `getInstance` is a static-style call made on the
/// un-init'ed class object (`createObject(…).getInstance("SHA256withRSA")`),
/// which is the shape vendored libraries use, so the receiver is left untouched
/// and a fresh configured instance is returned.
pub fn handle_java_signature(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match method {
        "init" => Ok(CfmlValue::strukt(shim(SIGNATURE_CLASS))),
        "getinstance" => {
            let requested = args.first().map(|a| a.as_string()).unwrap_or_default();
            if parse_algorithm(&requested).is_none() {
                return Err(no_such_algorithm(&requested));
            }
            let mut m = shim(SIGNATURE_CLASS);
            m.insert("__algorithm".to_string(), CfmlValue::string(requested));
            m.insert("__data".to_string(), CfmlValue::Binary(Vec::new()));
            Ok(CfmlValue::strukt(m))
        }
        "getalgorithm" => Ok(CfmlValue::string(field_string(object, "__algorithm"))),
        "initverify" | "initsign" => {
            let CfmlValue::Struct(state) = object else {
                return Ok(CfmlValue::Null);
            };
            let Some(key) = args.first() else {
                return Err(key_error("no key supplied"));
            };
            // Reset the accumulated input, as Java's init* does: the same
            // instance is routinely re-inited for a second verification.
            state.insert("__data".to_string(), CfmlValue::Binary(Vec::new()));
            state.insert("__key".to_string(), key.clone());
            state.insert(
                "__mode".to_string(),
                CfmlValue::string(
                    if method == "initsign" { "sign" } else { "verify" }.to_string(),
                ),
            );
            Ok(CfmlValue::Null)
        }
        "update" => {
            let CfmlValue::Struct(state) = object else {
                return Ok(CfmlValue::Null);
            };
            let mut data = field_bytes(object, "__data");
            if let Some(chunk) = args.first() {
                data.extend_from_slice(&bytes_of(chunk));
            }
            state.insert("__data".to_string(), CfmlValue::Binary(data));
            Ok(CfmlValue::Null)
        }
        "verify" => {
            let algorithm = field_string(object, "__algorithm");
            let Some(digest) = parse_algorithm(&algorithm) else {
                return Err(no_such_algorithm(&algorithm));
            };
            let key = match object {
                CfmlValue::Struct(s) => s.get("__key").unwrap_or(CfmlValue::Null),
                _ => CfmlValue::Null,
            };
            let public = rsa_public_from_key_shim(&key)?;
            let signature = args.first().map(bytes_of).unwrap_or_default();
            let (hashed, scheme) = digest_and_scheme(digest, &field_bytes(object, "__data"));
            // A failed verification is a VALUE, not an error: callers map false
            // onto their own typed error, and a throw here would escape that.
            Ok(CfmlValue::Bool(
                public.verify(scheme, &hashed, &signature).is_ok(),
            ))
        }
        "sign" => {
            let algorithm = field_string(object, "__algorithm");
            let Some(digest) = parse_algorithm(&algorithm) else {
                return Err(no_such_algorithm(&algorithm));
            };
            let key = match object {
                CfmlValue::Struct(s) => s.get("__key").unwrap_or(CfmlValue::Null),
                _ => CfmlValue::Null,
            };
            let private = rsa_private_from_key_shim(&key)?;
            let (hashed, scheme) = digest_and_scheme(digest, &field_bytes(object, "__data"));
            let signed = private.sign(scheme, &hashed).map_err(|e| {
                CfmlError::runtime(format!("java.security.SignatureException: {}", e))
            })?;
            Ok(CfmlValue::Binary(signed))
        }
        _ => Err(CfmlError::shim_unhandled(method)),
    }
}

/// `java.security.KeyFactory`. Like `Signature`, `getInstance` is called on the
/// un-init'ed class object; only "RSA" is accepted.
pub fn handle_java_keyfactory(method: &str, args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    match method {
        "init" => Ok(CfmlValue::strukt(shim(KEYFACTORY_CLASS))),
        "getinstance" => {
            let requested = args.first().map(|a| a.as_string()).unwrap_or_default();
            if !requested.eq_ignore_ascii_case("RSA") {
                return Err(no_such_algorithm(&requested));
            }
            let mut m = shim(KEYFACTORY_CLASS);
            m.insert("__algorithm".to_string(), CfmlValue::string("RSA".to_string()));
            Ok(CfmlValue::strukt(m))
        }
        "getalgorithm" => Ok(CfmlValue::string(field_string(object, "__algorithm"))),
        "generatepublic" => {
            let spec = args.first().cloned().unwrap_or(CfmlValue::Null);
            match field_string(&spec, "__java_class").as_str() {
                X509_SPEC_CLASS => {
                    // X509 SubjectPublicKeyInfo DER — what a PEM public key's
                    // base64 body decodes to.
                    let der = field_bytes(&spec, "__der");
                    let key = RsaPublicKey::from_public_key_der(&der).map_err(key_error)?;
                    Ok(public_key_value(
                        &key.n().to_bytes_be(),
                        &key.e().to_bytes_be(),
                    ))
                }
                RSA_PUBLIC_SPEC_CLASS => Ok(public_key_value(
                    &field_bytes(&spec, "__n"),
                    &field_bytes(&spec, "__e"),
                )),
                other => Err(key_error(format!(
                    "generatePublic does not accept a [{}] key spec",
                    if other.is_empty() { "non-spec value" } else { other }
                ))),
            }
        }
        "generateprivate" => {
            let spec = args.first().cloned().unwrap_or(CfmlValue::Null);
            if field_string(&spec, "__java_class") != PKCS8_SPEC_CLASS {
                return Err(key_error(
                    "generatePrivate requires a PKCS8EncodedKeySpec".to_string(),
                ));
            }
            let der = field_bytes(&spec, "__der");
            // Parsed eagerly so a malformed key fails HERE, where Java raises
            // InvalidKeySpecException, rather than later inside sign().
            RsaPrivateKey::from_pkcs8_der(&der).map_err(key_error)?;
            let mut m = shim(PRIVATE_KEY_CLASS);
            m.insert("__algorithm".to_string(), CfmlValue::string("RSA".to_string()));
            m.insert("__pkcs8".to_string(), CfmlValue::Binary(der));
            Ok(CfmlValue::strukt(m))
        }
        _ => Err(CfmlError::shim_unhandled(method)),
    }
}

/// The three key specs, plus the `PublicKey`/`PrivateKey` objects a
/// `KeyFactory` hands back. All are inert carriers — only `getAlgorithm()` and
/// `getEncoded()` are meaningful on them.
pub fn handle_java_key_object(
    class: &str,
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => match class {
            X509_SPEC_CLASS | PKCS8_SPEC_CLASS => {
                let mut m = shim(class);
                m.insert(
                    "__der".to_string(),
                    CfmlValue::Binary(args.first().map(bytes_of).unwrap_or_default()),
                );
                Ok(CfmlValue::strukt(m))
            }
            RSA_PUBLIC_SPEC_CLASS => {
                // RSAPublicKeySpec(BigInteger modulus, BigInteger exponent) —
                // the JWKS path. Both arrive as BigInteger shims.
                let mut m = shim(class);
                let magnitude = |i: usize| -> CfmlValue {
                    CfmlValue::Binary(
                        args.get(i)
                            .map(|v| field_bytes(v, "__magnitude"))
                            .unwrap_or_default(),
                    )
                };
                m.insert("__n".to_string(), magnitude(0));
                m.insert("__e".to_string(), magnitude(1));
                Ok(CfmlValue::strukt(m))
            }
            _ => Ok(CfmlValue::strukt(shim(class))),
        },
        "getalgorithm" => Ok(CfmlValue::string(field_string(object, "__algorithm"))),
        "getformat" => Ok(CfmlValue::string(
            match class {
                PUBLIC_KEY_CLASS | X509_SPEC_CLASS => "X.509",
                PRIVATE_KEY_CLASS | PKCS8_SPEC_CLASS => "PKCS#8",
                _ => "",
            }
            .to_string(),
        )),
        "getencoded" => match class {
            X509_SPEC_CLASS | PKCS8_SPEC_CLASS => {
                Ok(CfmlValue::Binary(field_bytes(object, "__der")))
            }
            PRIVATE_KEY_CLASS => Ok(CfmlValue::Binary(field_bytes(object, "__pkcs8"))),
            PUBLIC_KEY_CLASS => {
                use rsa::pkcs8::EncodePublicKey;
                let key = rsa_public_from_key_shim(object)?;
                let der = key.to_public_key_der().map_err(key_error)?;
                Ok(CfmlValue::Binary(der.as_bytes().to_vec()))
            }
            _ => Ok(CfmlValue::Null),
        },
        "getmodulus" => Ok(CfmlValue::Binary(field_bytes(object, "__n"))),
        "getpublicexponent" => Ok(CfmlValue::Binary(field_bytes(object, "__e"))),
        _ => Err(CfmlError::shim_unhandled(method)),
    }
}

/// `java.math.BigInteger`, only as far as the JWKS path needs it.
///
/// `init(signum, magnitude)` is **signum-magnitude, not two's-complement** —
/// the distinction that matters for a JWKS modulus, whose high bit is set: read
/// as two's-complement it would come out negative with the wrong `bitLength`.
pub fn handle_java_biginteger(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => {
            let mut m = shim(BIGINTEGER_CLASS);
            let (signum, magnitude) = match args.len() {
                // BigInteger(int signum, byte[] magnitude)
                n if n >= 2 => {
                    let s = args[0].as_string().trim().parse::<i64>().unwrap_or(1);
                    (s, bytes_of(&args[1]))
                }
                // BigInteger(String decimal) — the only other form vendored
                // code reaches for (exponents written as "65537").
                1 => {
                    let text = args[0].as_string();
                    let trimmed = text.trim();
                    let (sign, digits) = match trimmed.strip_prefix('-') {
                        Some(rest) => (-1, rest),
                        None => (1, trimmed.trim_start_matches('+')),
                    };
                    let parsed = BigUint::parse_bytes(digits.as_bytes(), 10)
                        .ok_or_else(|| key_error(format!("not a decimal integer: {}", text)))?;
                    let is_zero = parsed == BigUint::from(0u32);
                    (if is_zero { 0 } else { sign }, parsed.to_bytes_be())
                }
                _ => (0, Vec::new()),
            };
            m.insert("__signum".to_string(), CfmlValue::Int(signum));
            m.insert("__magnitude".to_string(), CfmlValue::Binary(magnitude));
            Ok(CfmlValue::strukt(m))
        }
        "bitlength" => {
            let magnitude = field_bytes(object, "__magnitude");
            let value = BigUint::from_bytes_be(&magnitude);
            Ok(CfmlValue::Int(value.bits() as i64))
        }
        "signum" => Ok(CfmlValue::Int(
            match object {
                CfmlValue::Struct(s) => s.get("__signum").map(|v| v.as_string()),
                _ => None,
            }
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(0),
        )),
        "tostring" => {
            let magnitude = field_bytes(object, "__magnitude");
            let value = BigUint::from_bytes_be(&magnitude);
            let signum = match object {
                CfmlValue::Struct(s) => s.get("__signum").map(|v| v.as_string()),
                _ => None,
            }
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(1);
            let text = value.to_str_radix(10);
            Ok(CfmlValue::string(if signum < 0 {
                format!("-{}", text)
            } else {
                text
            }))
        }
        "tobytearray" => Ok(CfmlValue::Binary(field_bytes(object, "__magnitude"))),
        _ => Err(CfmlError::shim_unhandled(method)),
    }
}

/// `createObject("java", …)` entry point for every class in this module.
pub fn construct(class_lower: &str) -> CfmlResult {
    match class_lower {
        SIGNATURE_CLASS => handle_java_signature("init", vec![], &CfmlValue::Null),
        KEYFACTORY_CLASS => handle_java_keyfactory("init", vec![], &CfmlValue::Null),
        BIGINTEGER_CLASS => Ok(CfmlValue::strukt(shim(BIGINTEGER_CLASS))),
        other => Ok(CfmlValue::strukt(shim(other))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algorithm_parsing_accepts_java_spellings_and_rejects_others() {
        assert!(parse_algorithm("SHA256withRSA").is_some());
        assert!(parse_algorithm("sha256WITHrsa").is_some());
        assert!(parse_algorithm("SHA-256withRSA").is_some());
        assert!(parse_algorithm("SHA1withRSA").is_some());
        // Non-RSA ciphers are NOT silently verified with PKCS#1 v1.5.
        assert!(parse_algorithm("SHA256withECDSA").is_none());
        assert!(parse_algorithm("SHA256withNOPE").is_none());
        assert!(parse_algorithm("SHA256").is_none());
    }

    #[test]
    fn biginteger_is_signum_magnitude_not_twos_complement() {
        // High bit set: a two's-complement read would go negative and report a
        // different bitLength.
        let magnitude = CfmlValue::Binary(vec![0xFFu8; 256]);
        let big = handle_java_biginteger("init", vec![CfmlValue::Int(1), magnitude], &CfmlValue::Null)
            .unwrap();
        let bits = handle_java_biginteger("bitlength", vec![], &big).unwrap();
        assert_eq!(bits.as_string(), "2048");
        let signum = handle_java_biginteger("signum", vec![], &big).unwrap();
        assert_eq!(signum.as_string(), "1");
    }

    #[test]
    fn biginteger_decimal_string_round_trips() {
        let big = handle_java_biginteger(
            "init",
            vec![CfmlValue::string("65537".to_string())],
            &CfmlValue::Null,
        )
        .unwrap();
        let text = handle_java_biginteger("tostring", vec![], &big).unwrap();
        assert_eq!(text.as_string(), "65537");
    }

    #[test]
    fn unknown_algorithm_raises_no_such_algorithm() {
        let err = handle_java_signature(
            "getinstance",
            vec![CfmlValue::string("SHA256withNOPE".to_string())],
            &CfmlValue::Null,
        )
        .unwrap_err();
        assert!(format!("{:?}", err).contains("NoSuchAlgorithm"));
    }
}
