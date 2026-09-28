//! TLS for LocalSend. Every device has a self-signed certificate whose SHA-256
//! is its fingerprint, so certificates aren't checked against any authority:
//! a peer is who it announced itself as when its certificate hashes to the
//! fingerprint it announced. Handshake signatures are still checked, so only
//! the holder of a certificate's key can present it.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, WebPkiSupportedAlgorithms};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, Error, ServerConfig, SignatureScheme,
};
use sha2::{Digest, Sha256};

/// A certificate's fingerprint: its SHA-256 in uppercase hex, as LocalSend
/// writes them.
pub fn fingerprint(cert: &[u8]) -> String {
    Sha256::digest(cert)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Our server: senders may present a certificate, which identifies them.
pub fn server_config(cert: &[u8], key: &[u8]) -> Result<ServerConfig, anyhow::Error> {
    let provider = provider();
    let algorithms = provider.signature_verification_algorithms;
    Ok(ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_client_cert_verifier(Arc::new(AnySender(algorithms)))
        .with_single_cert(certificate(cert), private_key(key))?)
}

/// Our requests to the device with `fingerprint`, presenting our certificate.
pub fn client_config(
    fingerprint: &str,
    cert: &[u8],
    key: &[u8],
) -> Result<ClientConfig, anyhow::Error> {
    let provider = provider();
    let verifier = PinnedReceiver {
        fingerprint: fingerprint.to_ascii_uppercase(),
        algorithms: provider.signature_verification_algorithms,
    };
    Ok(ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_client_auth_cert(certificate(cert), private_key(key))?)
}

fn certificate(der: &[u8]) -> Vec<CertificateDer<'static>> {
    vec![CertificateDer::from(der.to_vec())]
}

fn private_key(der: &[u8]) -> PrivateKeyDer<'static> {
    PrivateKeyDer::Pkcs8(der.to_vec().into())
}

/// Accepts the receiver whose certificate hashes to `fingerprint`.
#[derive(Debug)]
struct PinnedReceiver {
    fingerprint: String,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PinnedReceiver {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        if fingerprint(end_entity) == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(Error::General(
                "the receiver isn't the device that announced itself".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// Lets any sender present a certificate, or none. Whether its fingerprint
/// is trusted is decided later, by whoever handles the transfer.
#[derive(Debug)]
struct AnySender(WebPkiSupportedAlgorithms);

impl ClientCertVerifier for AnySender {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.0)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.0)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_schemes()
    }
}
