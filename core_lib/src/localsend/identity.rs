//! Our self-signed certificate, which our HTTPS server presents and our client
//! presents too (current LocalSend versions refuse uploads without one). Its
//! SHA-256 is our fingerprint, which peers use to recognise us and we use to
//! ignore our own announcements, so it is kept across restarts.

use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Arc;

use rustls::ServerConfig;

use super::tls;

const CERT_FILE: &str = "localsend-cert.der";
const KEY_FILE: &str = "localsend-key.der";

pub struct Identity {
    pub fingerprint: String,
    pub tls: Arc<ServerConfig>,
    /// DER, for our requests to peers, which present the same certificate.
    pub cert: Vec<u8>,
    pub key: Vec<u8>,
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

    Ok(Identity {
        fingerprint: tls::fingerprint(&cert),
        tls: Arc::new(tls::server_config(&cert, &key)?),
        cert,
        key,
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
