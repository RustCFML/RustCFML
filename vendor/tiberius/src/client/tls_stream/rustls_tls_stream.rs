// RustCFML: ported from tokio-rustls 0.24 / rustls 0.21 to tokio-rustls 0.26 /
// rustls 0.23 (the rustls the rest of RustCFML ships; 0.21 carries RUSTSEC
// advisories), with the aws-lc-rs crypto provider as RustCFML's PostgreSQL
// driver uses. Behaviour is unchanged: a CA file, trust-all, or the platform's
// native roots. See vendor/tiberius/RUSTCFML.txt.
use crate::{
    client::{config::Config, TrustConfig},
    error::IoErrorKind,
    Error,
};
use futures_util::io::{AsyncRead, AsyncWrite};
use std::{
    fs, io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio_rustls::{
    rustls::{
        client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        crypto::{self, CryptoProvider},
        pki_types::{pem::PemObject, CertificateDer, ServerName, UnixTime},
        ClientConfig, DigitallySignedStruct, Error as RustlsError, RootCertStore, SignatureScheme,
    },
    TlsConnector,
};
use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};
use tracing::{event, Level};

impl From<tokio_rustls::rustls::Error> for Error {
    fn from(e: tokio_rustls::rustls::Error) -> Self {
        crate::Error::Tls(e.to_string())
    }
}

pub(crate) struct TlsStream<S: AsyncRead + AsyncWrite + Unpin + Send>(
    Compat<tokio_rustls::client::TlsStream<Compat<S>>>,
);

/// `trustServerCertificate=true`: accept any certificate.
///
/// The handshake signatures are accepted too, as tiberius's rustls 0.21 code
/// did. Checking them means parsing the certificate with webpki, which refuses
/// the X.509 v1 certificate SQL Server generates for itself when none is
/// configured (`UnsupportedCertVersion`) — the very case this option exists
/// for. It would add nothing: whoever is on the other end chose the
/// certificate, so it only proves they hold the key for a certificate nobody
/// checked.
#[derive(Debug)]
struct NoCertVerifier(Arc<CryptoProvider>);

impl ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

fn get_server_name(config: &Config) -> crate::Result<ServerName<'static>> {
    match (ServerName::try_from(config.get_host().to_string()), &config.trust) {
        (Ok(sn), _) => Ok(sn),
        (Err(_), TrustConfig::TrustAll) => {
            Ok(ServerName::try_from("placeholder.domain.com").unwrap())
        }
        (Err(e), _) => Err(crate::Error::Tls(e.to_string())),
    }
}

fn invalid_input(message: String) -> Error {
    Error::Io {
        kind: IoErrorKind::InvalidInput,
        message,
    }
}

/// The CA certificate named by `trustServerCertificateCA`: one PEM/CRT
/// certificate, or a DER file.
fn read_ca_certificate(path: &std::path::Path) -> crate::Result<CertificateDer<'static>> {
    let buf = fs::read(path).map_err(|_| Error::Io {
        kind: IoErrorKind::InvalidData,
        message: "Could not read provided CA certificate!".to_string(),
    })?;
    let ext = path
        .extension()
        .map(|e| e.to_ascii_lowercase().to_string_lossy().into_owned());
    match ext.as_deref() {
        Some("pem") | Some("crt") => {
            let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(&buf)
                .collect::<Result<_, _>>()
                .map_err(|e| invalid_input(format!("{}: {e}", path.to_string_lossy())))?;
            if certs.len() != 1 {
                return Err(invalid_input(format!(
                    "Certificate file {} contain 0 or more than 1 certs",
                    path.to_string_lossy()
                )));
            }
            Ok(certs.into_iter().next().unwrap())
        }
        Some("der") => Ok(CertificateDer::from(buf)),
        _ => Err(invalid_input(
            "Provided CA certificate with unsupported file-extension! Supported types are pem, crt and der."
                .to_string(),
        )),
    }
}

/// The platform's root certificates, as rustls-native-certs loads them.
fn native_roots() -> crate::Result<RootCertStore> {
    let mut roots = RootCertStore::empty();
    let loaded = rustls_native_certs::load_native_certs();
    let (mut valid, mut invalid) = (0, 0);
    for cert in loaded.certs {
        match roots.add(cert) {
            Ok(_) => valid += 1,
            Err(err) => {
                event!(Level::DEBUG, "certificate parsing failed: {:?}", err);
                invalid += 1
            }
        }
    }
    event!(
        Level::TRACE,
        "with_native_roots processed {} valid and {} invalid certs",
        valid,
        invalid
    );
    if roots.is_empty() {
        return Err(Error::Tls(format!(
            "no CA certificates found{}",
            loaded
                .errors
                .first()
                .map(|e| format!(": {e}"))
                .unwrap_or_default()
        )));
    }
    Ok(roots)
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> TlsStream<S> {
    pub(super) async fn new(config: &Config, stream: S) -> crate::Result<Self> {
        event!(Level::INFO, "Performing a TLS handshake");

        let provider = Arc::new(crypto::aws_lc_rs::default_provider());
        let builder = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()?;

        let client_config = match &config.trust {
            TrustConfig::CaCertificateLocation(path) => {
                let mut cert_store = RootCertStore::empty();
                cert_store.add(read_ca_certificate(path)?)?;
                builder
                    .with_root_certificates(cert_store)
                    .with_no_client_auth()
            }
            TrustConfig::TrustAll => {
                event!(
                    Level::WARN,
                    "Trusting the server certificate without validation."
                );
                builder
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(NoCertVerifier(provider)))
                    .with_no_client_auth()
            }
            TrustConfig::Default => {
                event!(Level::INFO, "Using default trust configuration.");
                builder
                    .with_root_certificates(native_roots()?)
                    .with_no_client_auth()
            }
        };

        let connector = TlsConnector::from(Arc::new(client_config));

        let tls_stream = connector
            .connect(get_server_name(config)?, stream.compat())
            .await?;

        Ok(TlsStream(tls_stream.compat()))
    }

    pub(crate) fn get_mut(&mut self) -> &mut S {
        self.0.get_mut().get_mut().0.get_mut()
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> AsyncRead for TlsStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let inner = Pin::get_mut(self);
        Pin::new(&mut inner.0).poll_read(cx, buf)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> AsyncWrite for TlsStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let inner = Pin::get_mut(self);
        Pin::new(&mut inner.0).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let inner = Pin::get_mut(self);
        Pin::new(&mut inner.0).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let inner = Pin::get_mut(self);
        Pin::new(&mut inner.0).poll_close(cx)
    }
}
