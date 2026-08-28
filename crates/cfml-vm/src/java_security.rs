//! `java.security` asymmetric-crypto shims — `Signature`, `KeyFactory`, the RSA
//! key specs, and `java.math.BigInteger`.
//!
//! This is the surface vendored JWT libraries (jwt-cfml and friends) reach
//! through `createObject("java", …)` to verify and sign RS256/ES256 tokens.
//! Scope is deliberately the minimal tier that runs those libraries unmodified:
//! **RSA (RSASSA-PKCS1-v1_5) and ECDSA over the three NIST curves**, sign,
//! verify and key generation. `CertificateFactory` is still not shimmed — a
//! library asking for it gets the same "class is not supported" error as
//! before, rather than a half-working stub.
//!
//! ECDSA signing over P-256 and P-384 is **RFC 6979 deterministic**, so no
//! entropy is drawn and the wasm targets need no random backend. Two operations
//! genuinely need an RNG and are therefore native-only, throwing on wasm rather
//! than inventing a weak key or a weak nonce:
//! `KeyPairGenerator.generateKeyPair()`, and **P-521 signing** — `p521` 0.13 has
//! no RFC 6979 implementation yet and uses a random nonce. P-521 *verification*
//! needs no entropy and works everywhere.
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
pub const KEYPAIRGEN_CLASS: &str = "java.security.keypairgenerator";
pub const KEYPAIR_CLASS: &str = "java.security.keypair";
pub const ECGEN_SPEC_CLASS: &str = "java.security.spec.ecgenparameterspec";

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
            | KEYPAIRGEN_CLASS
            | KEYPAIR_CLASS
            | ECGEN_SPEC_CLASS
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

/// The cipher half of the same name. Keeping it in the parse result is what
/// stops an ECDSA algorithm from being verified with the RSA scheme (or the
/// reverse): both paths ask the same function, and it answers with both halves.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SigCipher {
    Rsa,
    Ecdsa,
}

/// Parse e.g. `"SHA256withRSA"` or `"SHA384withECDSA"` (case-insensitive,
/// Java's own spelling) into its digest and cipher. `None` for anything we do
/// not implement, so an unknown cipher fails loudly at `getInstance` time
/// rather than being signed with whichever scheme happens to be first.
fn parse_algorithm(algorithm: &str) -> Option<(SigDigest, SigCipher)> {
    let normalised = algorithm.to_ascii_uppercase().replace('-', "");
    let (digest, cipher) = normalised.split_once("WITH")?;
    let cipher = match cipher {
        "RSA" => SigCipher::Rsa,
        "ECDSA" | "EC" => SigCipher::Ecdsa,
        _ => return None,
    };
    let digest = match digest {
        "SHA" | "SHA1" => SigDigest::Sha1,
        "SHA256" => SigDigest::Sha256,
        "SHA384" => SigDigest::Sha384,
        "SHA512" => SigDigest::Sha512,
        _ => return None,
    };
    Some((digest, cipher))
}

/// Just the hash, for the ECDSA path — which signs a prehash rather than a
/// padded DigestInfo, so it needs the bare digest without the PKCS#1 scheme.
fn digest_bytes(digest: SigDigest, data: &[u8]) -> Vec<u8> {
    match digest {
        SigDigest::Sha1 => sha1::Sha1::digest(data).to_vec(),
        SigDigest::Sha256 => sha2::Sha256::digest(data).to_vec(),
        SigDigest::Sha384 => sha2::Sha384::digest(data).to_vec(),
        SigDigest::Sha512 => sha2::Sha512::digest(data).to_vec(),
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

// ── ECDSA over the NIST curves ───────────────────────────────────────────────
//
// The curve is NOT taken from the signature algorithm name: Java is perfectly
// happy to sign with SHA256withECDSA over a P-384 key, and the key's own
// encoding (SPKI / PKCS#8) already names its curve. So every entry point below
// discovers the curve by *parsing the key*, and the digest comes from the
// algorithm name independently — the same split Java makes.
//
// Signing goes through `sign_prehash`, not `sign`, for exactly that reason: it
// takes an already-computed hash of any length and applies the FIPS 186-4
// bits2int truncation, which is what lets SHA-256 pair with P-384 (and SHA-512
// with P-521, where the hash is SHORTER than the field).

/// A NIST prime curve, i.e. the three `ECGenParameterSpec` names the JWT
/// algorithms ES256/384/512 map onto.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EcCurve {
    P256,
    P384,
    P521,
}

impl EcCurve {
    /// The name Java's `ECGenParameterSpec` uses, and what `getCurveName()`
    /// style callers expect back.
    fn std_name(self) -> &'static str {
        match self {
            EcCurve::P256 => "secp256r1",
            EcCurve::P384 => "secp384r1",
            EcCurve::P521 => "secp521r1",
        }
    }

    /// Accept every spelling the JDK does for these three curves — the SECG
    /// name, the ANSI X9.62 alias for P-256, and the NIST "P-256" form.
    fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().replace('-', "").as_str() {
            "secp256r1" | "prime256v1" | "p256" | "nistp256" => Some(EcCurve::P256),
            "secp384r1" | "p384" | "nistp384" => Some(EcCurve::P384),
            "secp521r1" | "p521" | "nistp521" => Some(EcCurve::P521),
            _ => None,
        }
    }
}

fn signature_error(msg: impl std::fmt::Display) -> CfmlError {
    CfmlError::new(
        format!("java.security.SignatureException: {}", msg),
        CfmlErrorType::Custom("java.security.SignatureException".to_string()),
    )
}

/// Which curve an X.509 `SubjectPublicKeyInfo` carries, by parsing it. Also the
/// validity check: a key that parses as none of the three is rejected here,
/// where Java raises `InvalidKeySpecException`.
/// The SPKI DER behind a `PublicKey` shim built from an EC key.
fn ec_public_key_der(object: &CfmlValue) -> Result<Vec<u8>, CfmlError> {
    let der = field_bytes(object, "__der");
    if der.is_empty() {
        return Err(key_error(
            "an ECDSA signature needs an EC public key, but the key supplied is not one",
        ));
    }
    Ok(der)
}

fn ec_public_curve(der: &[u8]) -> Result<EcCurve, CfmlError> {
    if p256::PublicKey::from_public_key_der(der).is_ok() {
        return Ok(EcCurve::P256);
    }
    if p384::PublicKey::from_public_key_der(der).is_ok() {
        return Ok(EcCurve::P384);
    }
    if p521::PublicKey::from_public_key_der(der).is_ok() {
        return Ok(EcCurve::P521);
    }
    Err(key_error(
        "not an EC public key on P-256, P-384 or P-521",
    ))
}

/// The same, for a PKCS#8 private key.
fn ec_private_curve(der: &[u8]) -> Result<EcCurve, CfmlError> {
    if p256::SecretKey::from_pkcs8_der(der).is_ok() {
        return Ok(EcCurve::P256);
    }
    if p384::SecretKey::from_pkcs8_der(der).is_ok() {
        return Ok(EcCurve::P384);
    }
    if p521::SecretKey::from_pkcs8_der(der).is_ok() {
        return Ok(EcCurve::P521);
    }
    Err(key_error(
        "not an EC private key on P-256, P-384 or P-521",
    ))
}

/// Sign a prehash, returning the **DER** encoding Java's `Signature.sign()`
/// hands back (`SEQUENCE { INTEGER r, INTEGER s }`) — not the fixed-width r‖s
/// form. Callers that want raw coordinates re-encode it themselves, which is
/// precisely what the vendored ES256 libraries do.
fn ec_sign(pkcs8: &[u8], hashed: &[u8]) -> Result<Vec<u8>, CfmlError> {
    use p256::ecdsa::signature::hazmat::PrehashSigner;
    if let Ok(sk) = p256::SecretKey::from_pkcs8_der(pkcs8) {
        let signer = p256::ecdsa::SigningKey::from(&sk);
        let sig: p256::ecdsa::Signature =
            signer.sign_prehash(hashed).map_err(signature_error)?;
        return Ok(sig.to_der().as_bytes().to_vec());
    }
    if let Ok(sk) = p384::SecretKey::from_pkcs8_der(pkcs8) {
        let signer = p384::ecdsa::SigningKey::from(&sk);
        let sig: p384::ecdsa::Signature =
            signer.sign_prehash(hashed).map_err(signature_error)?;
        return Ok(sig.to_der().as_bytes().to_vec());
    }
    if let Ok(sk) = p521::SecretKey::from_pkcs8_der(pkcs8) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = sk;
            return Err(no_p521_signing_on_wasm());
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            // p521 wraps its ECDSA keys in newtypes rather than re-exporting
            // the generic `ecdsa` ones, so it has no `From<&SecretKey>` — go
            // through the scalar bytes instead.
            let signer =
                p521::ecdsa::SigningKey::from_slice(&sk.to_bytes()).map_err(key_error)?;
            let sig: p521::ecdsa::Signature =
                signer.sign_prehash(hashed).map_err(signature_error)?;
            return Ok(sig.to_der().as_bytes().to_vec());
        }
    }
    Err(key_error("not an EC private key on P-256, P-384 or P-521"))
}

/// Verify a DER-encoded ECDSA signature against a prehash.
///
/// A malformed or non-matching signature is `Ok(false)`, never an error — the
/// same contract `Signature.verify()` keeps for RSA, and the reason callers can
/// map a failure onto their own typed error instead of an engine throw.
fn ec_verify(spki: &[u8], hashed: &[u8], sig_der: &[u8]) -> Result<bool, CfmlError> {
    use p256::ecdsa::signature::hazmat::PrehashVerifier;
    match ec_public_curve(spki)? {
        EcCurve::P256 => {
            let key = p256::PublicKey::from_public_key_der(spki).map_err(key_error)?;
            let verifier = p256::ecdsa::VerifyingKey::from(&key);
            let Ok(sig) = p256::ecdsa::Signature::from_der(sig_der) else {
                return Ok(false);
            };
            Ok(verifier.verify_prehash(hashed, &sig).is_ok())
        }
        EcCurve::P384 => {
            let key = p384::PublicKey::from_public_key_der(spki).map_err(key_error)?;
            let verifier = p384::ecdsa::VerifyingKey::from(&key);
            let Ok(sig) = p384::ecdsa::Signature::from_der(sig_der) else {
                return Ok(false);
            };
            Ok(verifier.verify_prehash(hashed, &sig).is_ok())
        }
        EcCurve::P521 => {
            use p256::elliptic_curve::sec1::ToEncodedPoint;
            let key = p521::PublicKey::from_public_key_der(spki).map_err(key_error)?;
            // Same newtype story as signing: build from the SEC1 point.
            let verifier =
                p521::ecdsa::VerifyingKey::from_sec1_bytes(key.to_encoded_point(false).as_bytes())
                    .map_err(key_error)?;
            let Ok(sig) = p521::ecdsa::Signature::from_der(sig_der) else {
                return Ok(false);
            };
            Ok(verifier.verify_prehash(hashed, &sig).is_ok())
        }
    }
}

/// Minimal DER tag-length-value.
fn der_tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else if len < 0x100 {
        out.extend_from_slice(&[0x81, len as u8]);
    } else {
        out.extend_from_slice(&[0x82, (len >> 8) as u8, (len & 0xFF) as u8]);
    }
    out.extend_from_slice(content);
    out
}

/// PKCS#8 for an EC private key **in the JDK's shape**.
///
/// This is deliberately not `SecretKey::to_pkcs8_der()`. RustCrypto (like
/// OpenSSL) fills in both optional fields of the inner `ECPrivateKey` — the
/// curve parameters and the public point — while SunEC omits both, since the
/// enclosing `AlgorithmIdentifier` already names the curve. Both parse, but the
/// encodings differ in LENGTH (P-256: 138 bytes vs 67), and real CFML libraries
/// validate a key by its byte length — the cfsignatures module the Preside
/// ReadyIntelligence extension bundles rejects any EC signing key that is not
/// 67/80/98 bytes. Emitting the OpenSSL shape made `validateSigningKey()`
/// return false for a key we had just generated ourselves.
///
///   PrivateKeyInfo ::= SEQUENCE {
///     version           INTEGER (0),
///     algorithm         SEQUENCE { id-ecPublicKey OID, namedCurve OID },
///     privateKey        OCTET STRING wrapping
///        ECPrivateKey ::= SEQUENCE { version INTEGER (1), privateKey OCTET STRING } }
fn jdk_ec_pkcs8(curve: EcCurve, scalar: &[u8]) -> Vec<u8> {
    // OID 1.2.840.10045.2.1 — id-ecPublicKey.
    const EC_PUBLIC_KEY_OID: &[u8] = &[0x06, 0x07, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01];
    let curve_oid: &[u8] = match curve {
        // 1.2.840.10045.3.1.7
        EcCurve::P256 => &[0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07],
        // 1.3.132.0.34
        EcCurve::P384 => &[0x06, 0x05, 0x2B, 0x81, 0x04, 0x00, 0x22],
        // 1.3.132.0.35
        EcCurve::P521 => &[0x06, 0x05, 0x2B, 0x81, 0x04, 0x00, 0x23],
    };

    let mut inner = vec![0x02, 0x01, 0x01]; // ECPrivateKey version = 1
    inner.extend_from_slice(&der_tlv(0x04, scalar));
    let ec_private_key = der_tlv(0x30, &inner);

    let mut algorithm = EC_PUBLIC_KEY_OID.to_vec();
    algorithm.extend_from_slice(curve_oid);

    let mut body = vec![0x02, 0x01, 0x00]; // PrivateKeyInfo version = 0
    body.extend_from_slice(&der_tlv(0x30, &algorithm));
    body.extend_from_slice(&der_tlv(0x04, &ec_private_key));
    der_tlv(0x30, &body)
}

/// Fresh EC key pair as `(PKCS#8 private, X.509 public)` DER — the two encodings
/// `KeyPair.getPrivate().getEncoded()` / `getPublic().getEncoded()` return.
#[cfg(not(target_arch = "wasm32"))]
fn ec_generate(curve: EcCurve) -> Result<(Vec<u8>, Vec<u8>), CfmlError> {
    use rsa::pkcs8::EncodePublicKey;
    let mut rng = rand_core::OsRng;
    macro_rules! gen {
        ($c:ident) => {{
            let secret = $c::SecretKey::random(&mut rng);
            let private = jdk_ec_pkcs8(curve, &secret.to_bytes());
            let public = secret
                .public_key()
                .to_public_key_der()
                .map_err(key_error)?
                .as_bytes()
                .to_vec();
            (private, public)
        }};
    }
    Ok(match curve {
        EcCurve::P256 => gen!(p256),
        EcCurve::P384 => gen!(p384),
        EcCurve::P521 => gen!(p521),
    })
}

/// Fresh RSA key pair in the same two encodings. Slow by nature (prime search),
/// which is why it is only ever reached from an explicit `generateKeyPair()`.
#[cfg(not(target_arch = "wasm32"))]
fn rsa_generate(bits: usize) -> Result<(Vec<u8>, Vec<u8>), CfmlError> {
    use rsa::pkcs8::{EncodePrivateKey, EncodePublicKey};
    let mut rng = rand_core::OsRng;
    let secret = RsaPrivateKey::new(&mut rng, bits)
        .map_err(|e| key_error(format!("RSA key generation failed: {}", e)))?;
    let private = secret.to_pkcs8_der().map_err(key_error)?.as_bytes().to_vec();
    let public = secret
        .to_public_key()
        .to_public_key_der()
        .map_err(key_error)?
        .as_bytes()
        .to_vec();
    Ok((private, public))
}

/// P-521 signing draws a random nonce (see the module header), so it is subject
/// to the same wasm limitation as key generation.
#[cfg(target_arch = "wasm32")]
fn no_p521_signing_on_wasm() -> CfmlError {
    CfmlError::runtime(
        "SHA512withECDSA signing over P-521 is not available on the wasm build: \
         P-521 needs a random nonce and there is no OS entropy source. \
         Verification, and signing over P-256/P-384, are unaffected."
            .to_string(),
    )
}

/// `KeyPairGenerator.generateKeyPair()` needs real entropy, and wasm builds
/// deliberately carry no random backend (see the module header). Throwing here
/// is the honest answer; the alternative would be a predictable key.
#[cfg(target_arch = "wasm32")]
fn no_keygen_on_wasm() -> CfmlError {
    CfmlError::runtime(
        "java.security.KeyPairGenerator.generateKeyPair() is not available on the \
         wasm build: it has no OS entropy source. Generate the key pair elsewhere \
         and supply it as PEM."
            .to_string(),
    )
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
            // The JDK refuses a mismatched key HERE, not at sign()/verify()
            // time, and as InvalidKeyException. Deferring it would surface the
            // wrong exception type a page later.
            let signature_algorithm = field_string(object, "__algorithm");
            if let Some((_, cipher)) = parse_algorithm(&signature_algorithm) {
                let key_algorithm = field_string(key, "__algorithm");
                let matches = match cipher {
                    SigCipher::Rsa => key_algorithm.eq_ignore_ascii_case("RSA"),
                    SigCipher::Ecdsa => key_algorithm.eq_ignore_ascii_case("EC"),
                };
                if !matches {
                    return Err(CfmlError::new(
                        format!(
                            "No installed provider supports this key: a [{}] key cannot be used with [{}]",
                            if key_algorithm.is_empty() { "non-key value" } else { &key_algorithm },
                            signature_algorithm
                        ),
                        CfmlErrorType::Custom("java.security.InvalidKeyException".to_string()),
                    ));
                }
            }
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
            let Some((digest, cipher)) = parse_algorithm(&algorithm) else {
                return Err(no_such_algorithm(&algorithm));
            };
            let key = match object {
                CfmlValue::Struct(s) => s.get("__key").unwrap_or(CfmlValue::Null),
                _ => CfmlValue::Null,
            };
            let signature = args.first().map(bytes_of).unwrap_or_default();
            let data = field_bytes(object, "__data");
            // A failed verification is a VALUE, not an error: callers map false
            // onto their own typed error, and a throw here would escape that.
            Ok(CfmlValue::Bool(match cipher {
                SigCipher::Rsa => {
                    let public = rsa_public_from_key_shim(&key)?;
                    let (hashed, scheme) = digest_and_scheme(digest, &data);
                    public.verify(scheme, &hashed, &signature).is_ok()
                }
                SigCipher::Ecdsa => {
                    let spki = ec_public_key_der(&key)?;
                    ec_verify(&spki, &digest_bytes(digest, &data), &signature)?
                }
            }))
        }
        "sign" => {
            let algorithm = field_string(object, "__algorithm");
            let Some((digest, cipher)) = parse_algorithm(&algorithm) else {
                return Err(no_such_algorithm(&algorithm));
            };
            let key = match object {
                CfmlValue::Struct(s) => s.get("__key").unwrap_or(CfmlValue::Null),
                _ => CfmlValue::Null,
            };
            let data = field_bytes(object, "__data");
            let signed = match cipher {
                SigCipher::Rsa => {
                    let private = rsa_private_from_key_shim(&key)?;
                    let (hashed, scheme) = digest_and_scheme(digest, &data);
                    private.sign(scheme, &hashed).map_err(signature_error)?
                }
                SigCipher::Ecdsa => ec_sign(
                    &field_bytes(&key, "__pkcs8"),
                    &digest_bytes(digest, &data),
                )?,
            };
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
            let Some(algorithm) = key_algorithm(&requested) else {
                return Err(no_such_algorithm(&requested));
            };
            let mut m = shim(KEYFACTORY_CLASS);
            m.insert(
                "__algorithm".to_string(),
                CfmlValue::string(algorithm.to_string()),
            );
            Ok(CfmlValue::strukt(m))
        }
        "getalgorithm" => Ok(CfmlValue::string(field_string(object, "__algorithm"))),
        "generatepublic" => {
            let ec = field_string(object, "__algorithm").eq_ignore_ascii_case("EC");
            let spec = args.first().cloned().unwrap_or(CfmlValue::Null);
            match field_string(&spec, "__java_class").as_str() {
                X509_SPEC_CLASS => {
                    // X509 SubjectPublicKeyInfo DER — what a PEM public key's
                    // base64 body decodes to.
                    let der = field_bytes(&spec, "__der");
                    if ec {
                        return Ok(ec_public_key_value(der)?);
                    }
                    let key = RsaPublicKey::from_public_key_der(&der).map_err(key_error)?;
                    Ok(public_key_value(
                        &key.n().to_bytes_be(),
                        &key.e().to_bytes_be(),
                    ))
                }
                RSA_PUBLIC_SPEC_CLASS if !ec => Ok(public_key_value(
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
            if field_string(object, "__algorithm").eq_ignore_ascii_case("EC") {
                return ec_private_key_value(der);
            }
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

/// Normalise the algorithm a `KeyFactory`/`KeyPairGenerator` is asked for.
///
/// `"ECDSA"` is deliberately NOT accepted: probed against a real JDK,
/// `KeyFactory.getInstance("ECDSA")` throws `NoSuchAlgorithmException` — only
/// `Signature` spells the cipher that way. Accepting it here would let code
/// that works on RustCFML fail on every other engine.
fn key_algorithm(requested: &str) -> Option<&'static str> {
    if requested.eq_ignore_ascii_case("RSA") {
        Some("RSA")
    } else if requested.eq_ignore_ascii_case("EC") {
        Some("EC")
    } else {
        None
    }
}

fn invalid_algorithm_parameter(msg: impl std::fmt::Display) -> CfmlError {
    CfmlError::new(
        format!("{}", msg),
        CfmlErrorType::Custom("java.security.InvalidAlgorithmParameterException".to_string()),
    )
}

/// An EC `PublicKey` shim. The SPKI DER is kept verbatim (it names the curve),
/// and parsed eagerly so a bad key fails at `generatePublic()` — where Java
/// raises `InvalidKeySpecException` — rather than inside `verify()`.
fn ec_public_key_value(der: Vec<u8>) -> CfmlResult {
    let curve = ec_public_curve(&der)?;
    let mut m = shim(PUBLIC_KEY_CLASS);
    m.insert("__algorithm".to_string(), CfmlValue::string("EC".to_string()));
    m.insert(
        "__curve".to_string(),
        CfmlValue::string(curve.std_name().to_string()),
    );
    m.insert("__der".to_string(), CfmlValue::Binary(der));
    Ok(CfmlValue::strukt(m))
}

/// The private half, same rules.
fn ec_private_key_value(der: Vec<u8>) -> CfmlResult {
    let curve = ec_private_curve(&der)?;
    let mut m = shim(PRIVATE_KEY_CLASS);
    m.insert("__algorithm".to_string(), CfmlValue::string("EC".to_string()));
    m.insert(
        "__curve".to_string(),
        CfmlValue::string(curve.std_name().to_string()),
    );
    m.insert("__pkcs8".to_string(), CfmlValue::Binary(der));
    Ok(CfmlValue::strukt(m))
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
            ECGEN_SPEC_CLASS => {
                // ECGenParameterSpec(String stdName) is an inert name holder:
                // probed against a real JDK, even "not-a-curve" constructs
                // happily, and the rejection lands at
                // KeyPairGenerator.initialize(). Validating here instead would
                // throw a page too early, with the wrong exception type.
                let mut m = shim(class);
                m.insert(
                    "__curve".to_string(),
                    CfmlValue::string(args.first().map(|a| a.as_string()).unwrap_or_default()),
                );
                Ok(CfmlValue::strukt(m))
            }
            _ => Ok(CfmlValue::strukt(shim(class))),
        },
        "getname" if class == ECGEN_SPEC_CLASS => {
            Ok(CfmlValue::string(field_string(object, "__curve")))
        }
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
                // An EC public key carries its SPKI verbatim; only the RSA path
                // has to re-encode from the modulus/exponent pair it stores.
                let der = field_bytes(object, "__der");
                if !der.is_empty() {
                    return Ok(CfmlValue::Binary(der));
                }
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

/// `java.security.KeyPairGenerator`. Same static-style `getInstance` shape as
/// `Signature`/`KeyFactory`, then `initialize(…)` — which is overloaded in Java
/// on an `int` key size (RSA) or an `AlgorithmParameterSpec` (EC), so both are
/// accepted and stored side by side.
pub fn handle_java_keypairgenerator(
    method: &str,
    args: Vec<CfmlValue>,
    object: &CfmlValue,
) -> CfmlResult {
    match method {
        "init" => Ok(CfmlValue::strukt(shim(KEYPAIRGEN_CLASS))),
        "getinstance" => {
            let requested = args.first().map(|a| a.as_string()).unwrap_or_default();
            let Some(algorithm) = key_algorithm(&requested) else {
                return Err(no_such_algorithm(&requested));
            };
            let mut m = shim(KEYPAIRGEN_CLASS);
            m.insert(
                "__algorithm".to_string(),
                CfmlValue::string(algorithm.to_string()),
            );
            Ok(CfmlValue::strukt(m))
        }
        "getalgorithm" => Ok(CfmlValue::string(field_string(object, "__algorithm"))),
        "initialize" => {
            let CfmlValue::Struct(state) = object else {
                return Ok(CfmlValue::Null);
            };
            let Some(arg) = args.first() else {
                return Ok(CfmlValue::Null);
            };
            // ECGenParameterSpec — the EC path, and where an unsupported curve
            // is refused (the JDK's own error, verbatim down to the wording).
            if field_string(arg, "__java_class") == ECGEN_SPEC_CLASS {
                let name = field_string(arg, "__curve");
                if EcCurve::from_name(&name).is_none() {
                    return Err(invalid_algorithm_parameter(format!(
                        "Curve not supported: {}",
                        name
                    )));
                }
                state.insert("__curve".to_string(), CfmlValue::string(name));
                return Ok(CfmlValue::Null);
            }
            // Otherwise an int key size (RSA).
            let bits = arg.as_string().trim().parse::<i64>().unwrap_or(0);
            if bits <= 0 {
                return Err(CfmlError::runtime(format!(
                    "java.security.InvalidAlgorithmParameterException: KeyPairGenerator.initialize() \
                     takes a key size or an ECGenParameterSpec, not [{}]",
                    arg.as_string()
                )));
            }
            state.insert("__size".to_string(), CfmlValue::Int(bits));
            Ok(CfmlValue::Null)
        }
        "generatekeypair" | "genkeypair" => {
            let algorithm = field_string(object, "__algorithm");
            #[cfg(target_arch = "wasm32")]
            {
                let _ = &algorithm;
                Err(no_keygen_on_wasm())
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let (private, public) = if algorithm.eq_ignore_ascii_case("EC") {
                    // An uninitialised EC generator defaults to P-384, not
                    // P-256 — probed against a real JDK (an 80-byte PKCS#8).
                    let name = field_string(object, "__curve");
                    let curve = if name.is_empty() {
                        EcCurve::P384
                    } else {
                        EcCurve::from_name(&name)
                            .ok_or_else(|| invalid_algorithm_parameter(
                                format!("Curve not supported: {}", name),
                            ))?
                    };
                    ec_generate(curve)?
                } else {
                    // ...and an uninitialised RSA generator to 3072 bits (a
                    // 422-byte SubjectPublicKeyInfo), not the 2048 it used to.
                    let bits = match object {
                        CfmlValue::Struct(s) => s
                            .get("__size")
                            .and_then(|v| v.as_string().trim().parse::<usize>().ok()),
                        _ => None,
                    }
                    .unwrap_or(3072);
                    rsa_generate(bits)?
                };
                let mut m = shim(KEYPAIR_CLASS);
                m.insert("__algorithm".to_string(), CfmlValue::string(algorithm));
                m.insert("__pkcs8".to_string(), CfmlValue::Binary(private));
                m.insert("__der".to_string(), CfmlValue::Binary(public));
                Ok(CfmlValue::strukt(m))
            }
        }
        _ => Err(CfmlError::shim_unhandled(method)),
    }
}

/// `java.security.KeyPair` — a pair of getters over the two encodings the
/// generator produced.
pub fn handle_java_keypair(method: &str, _args: Vec<CfmlValue>, object: &CfmlValue) -> CfmlResult {
    let algorithm = field_string(object, "__algorithm");
    match method {
        "init" => Ok(CfmlValue::strukt(shim(KEYPAIR_CLASS))),
        "getprivate" => {
            let der = field_bytes(object, "__pkcs8");
            if algorithm.eq_ignore_ascii_case("EC") {
                return ec_private_key_value(der);
            }
            let mut m = shim(PRIVATE_KEY_CLASS);
            m.insert("__algorithm".to_string(), CfmlValue::string(algorithm));
            m.insert("__pkcs8".to_string(), CfmlValue::Binary(der));
            Ok(CfmlValue::strukt(m))
        }
        "getpublic" => {
            let der = field_bytes(object, "__der");
            if algorithm.eq_ignore_ascii_case("EC") {
                return ec_public_key_value(der);
            }
            let key = RsaPublicKey::from_public_key_der(&der).map_err(key_error)?;
            Ok(public_key_value(
                &key.n().to_bytes_be(),
                &key.e().to_bytes_be(),
            ))
        }
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
        KEYPAIRGEN_CLASS => handle_java_keypairgenerator("init", vec![], &CfmlValue::Null),
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
        assert!(parse_algorithm("SHA256withNOPE").is_none());
        assert!(parse_algorithm("SHA256").is_none());
    }

    #[test]
    fn cipher_travels_with_the_digest() {
        // The cipher half decides the SCHEME. Losing it is how an ECDSA
        // algorithm would end up verified with PKCS#1 v1.5.
        assert!(matches!(
            parse_algorithm("SHA256withRSA"),
            Some((SigDigest::Sha256, SigCipher::Rsa))
        ));
        assert!(matches!(
            parse_algorithm("SHA384withECDSA"),
            Some((SigDigest::Sha384, SigCipher::Ecdsa))
        ));
        assert!(matches!(
            parse_algorithm("sha512WITHecdsa"),
            Some((SigDigest::Sha512, SigCipher::Ecdsa))
        ));
    }

    #[test]
    fn curve_names_accept_every_jdk_spelling() {
        for name in ["secp256r1", "prime256v1", "P-256", "NISTP256"] {
            assert_eq!(EcCurve::from_name(name), Some(EcCurve::P256), "{}", name);
        }
        assert_eq!(EcCurve::from_name("secp384r1"), Some(EcCurve::P384));
        assert_eq!(EcCurve::from_name("secp521r1"), Some(EcCurve::P521));
        // secp256k1 is a real curve we do NOT implement — it must not be
        // silently answered with P-256.
        assert_eq!(EcCurve::from_name("secp256k1"), None);
    }

    /// Round-trip through the shim surface, curve by curve: generate, sign,
    /// verify, and confirm a tampered payload fails. The digest is paired with
    /// the curve the way the JWT ES* algorithms do.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn ecdsa_round_trips_on_every_curve() {
        for (curve, digest) in [
            (EcCurve::P256, SigDigest::Sha256),
            (EcCurve::P384, SigDigest::Sha384),
            (EcCurve::P521, SigDigest::Sha512),
        ] {
            let (pkcs8, spki) = ec_generate(curve).unwrap();
            assert_eq!(ec_private_curve(&pkcs8).unwrap(), curve);
            assert_eq!(ec_public_curve(&spki).unwrap(), curve);

            let payload = b"eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiIxIn0";
            let sig = ec_sign(&pkcs8, &digest_bytes(digest, payload)).unwrap();
            // Java hands back a DER SEQUENCE, not raw r||s — vendored ES256
            // libraries re-encode it themselves and would mis-parse the other.
            assert_eq!(sig[0], 0x30, "signature must be a DER SEQUENCE");
            assert!(ec_verify(&spki, &digest_bytes(digest, payload), &sig).unwrap());
            assert!(!ec_verify(&spki, &digest_bytes(digest, b"tampered"), &sig).unwrap());
        }
    }

    /// The generated PKCS#8 must be the JDK's encoding, byte-length included:
    /// real CFML libraries gate on exactly these numbers.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn generated_ec_private_keys_use_the_jdk_pkcs8_length() {
        for (curve, private_len, public_len) in [
            (EcCurve::P256, 67, 91),
            (EcCurve::P384, 80, 120),
            (EcCurve::P521, 98, 158),
        ] {
            let (pkcs8, spki) = ec_generate(curve).unwrap();
            assert_eq!(pkcs8.len(), private_len, "{:?} private", curve);
            assert_eq!(spki.len(), public_len, "{:?} public", curve);
            // Still a valid key, not just the right size.
            assert_eq!(ec_private_curve(&pkcs8).unwrap(), curve);
        }
    }

    /// ...and the OpenSSL shape (both optional fields present) must still LOAD —
    /// keys arrive from everywhere, only our own output is normalised.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn openssl_shaped_ec_pkcs8_still_loads() {
        use rsa::pkcs8::EncodePrivateKey;
        let secret = p256::SecretKey::random(&mut rand_core::OsRng);
        let openssl_shape = secret.to_pkcs8_der().unwrap().as_bytes().to_vec();
        assert!(openssl_shape.len() > 67, "expected the longer encoding");
        assert_eq!(ec_private_curve(&openssl_shape).unwrap(), EcCurve::P256);
        // Both encodings of the SAME key sign identically (RFC 6979).
        let hashed = digest_bytes(SigDigest::Sha256, b"payload");
        let jdk_shape = jdk_ec_pkcs8(EcCurve::P256, &secret.to_bytes());
        assert_eq!(
            ec_sign(&jdk_shape, &hashed).unwrap(),
            ec_sign(&openssl_shape, &hashed).unwrap()
        );
    }

    /// A P-384 key signed with SHA-256 — legal in Java, and the reason the
    /// curve is read from the KEY rather than from the algorithm name.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn digest_and_curve_are_independent() {
        let (pkcs8, spki) = ec_generate(EcCurve::P384).unwrap();
        let hashed = digest_bytes(SigDigest::Sha256, b"payload");
        let sig = ec_sign(&pkcs8, &hashed).unwrap();
        assert!(ec_verify(&spki, &hashed, &sig).unwrap());
    }

    /// Garbage in the signature slot is `false`, not an error — the same
    /// contract the RSA path keeps.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn malformed_signature_verifies_false_rather_than_throwing() {
        let (_, spki) = ec_generate(EcCurve::P256).unwrap();
        let hashed = digest_bytes(SigDigest::Sha256, b"payload");
        assert!(!ec_verify(&spki, &hashed, b"not a signature").unwrap());
    }

    /// An RSA key must not be accepted by the EC path (or vice versa).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn rsa_and_ec_keys_do_not_cross_over() {
        let (rsa_pkcs8, rsa_spki) = rsa_generate(2048).unwrap();
        assert!(ec_public_curve(&rsa_spki).is_err());
        assert!(ec_private_curve(&rsa_pkcs8).is_err());
        let (ec_pkcs8, ec_spki) = ec_generate(EcCurve::P256).unwrap();
        assert!(RsaPublicKey::from_public_key_der(&ec_spki).is_err());
        assert!(RsaPrivateKey::from_pkcs8_der(&ec_pkcs8).is_err());
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
