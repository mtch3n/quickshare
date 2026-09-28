//! The self-signed certificate our HTTPS server presents. LocalSend doesn't
//! verify it; its SHA-256 is our fingerprint, which peers use to recognise us
//! and we use to ignore our own announcements, so it is kept across restarts.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;

use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};

const CERT_FILE: &str = "localsend-cert.der";
const KEY_FILE: &str = "localsend-key.der";

pub struct Identity {
    pub fingerprint: String,
    pub tls: Arc<ServerConfig>,
}

pub fn load_or_create(dir: Option<&Path>) -> Result<Identity, anyhow::Error> {
    let (cert, key) = match dir.and_then(load) {
        Some(pair) => pair,
        None => {
            let pair = create()?;
            if let Some(dir) = dir
                && let Err(e) = save(dir, &pair)
            {
                warn!("LocalSend: couldn't save the certificate: {e}");
            }
            pair
        }
    };

    let fingerprint = Sha256::digest(&cert)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let tls =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert)],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
            )?;

    Ok(Identity {
        fingerprint,
        tls: Arc::new(tls),
    })
}

fn load(dir: &Path) -> Option<(Vec<u8>, Vec<u8>)> {
    let cert = std::fs::read(dir.join(CERT_FILE)).ok()?;
    let key = std::fs::read(dir.join(KEY_FILE)).ok()?;
    Some((cert, key))
}

fn create() -> Result<(Vec<u8>, Vec<u8>), anyhow::Error> {
    let certified = rcgen::generate_simple_self_signed(vec!["localsend".to_owned()])?;
    Ok((
        certified.cert.der().to_vec(),
        certified.signing_key.serialize_der(),
    ))
}

fn save(dir: &Path, (cert, key): &(Vec<u8>, Vec<u8>)) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(CERT_FILE), cert)?;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(dir.join(KEY_FILE))?
        .write_all(key)
}
