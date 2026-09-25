//! UKEY2 key agreement and SecureMessage (AES-256-CBC + HMAC-SHA256) primitives
//! shared by inbound and outbound connections.

use aes::Aes256;
use anyhow::{anyhow, bail};
use cbc::cipher::block_padding::Pkcs7;
use cbc::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use hkdf::Hkdf;
use hmac::{Hmac, KeyInit, Mac};
use p256::ecdh::diffie_hellman;
use p256::elliptic_curve::Generate;
use p256::elliptic_curve::sec1::{FromSec1Point, ToSec1Point};
use p256::{PublicKey, Sec1Point, SecretKey};
use sha2::{Digest, Sha256};

use crate::utils::to_four_digit_string;

type HmacSha256 = Hmac<Sha256>;

const D2D_SALT: [u8; 32] = [
    0x82, 0xAA, 0x55, 0xA0, 0xD3, 0x97, 0xF8, 0x83, 0x46, 0xCA, 0x1C, 0xEE, 0x8D, 0x39, 0x09, 0xB9,
    0x5F, 0x13, 0xFA, 0x7D, 0xEB, 0x1D, 0x4A, 0xB3, 0x83, 0x76, 0xB8, 0x25, 0x6D, 0xA8, 0x55, 0x10,
];

const SECURE_MESSAGE_SALT: [u8; 32] = [
    0xBF, 0x9D, 0x2A, 0x53, 0xC6, 0x36, 0x16, 0xD7, 0x5D, 0xB0, 0xA7, 0x16, 0x5B, 0x91, 0xC1, 0xEF,
    0x73, 0xE5, 0x37, 0xF2, 0x42, 0x74, 0x05, 0xFA, 0x23, 0x61, 0x0A, 0x4B, 0xE6, 0x57, 0x64, 0x2E,
];

/// Which side of the UKEY2 handshake we are on.
pub enum Role {
    Client,
    Server,
}

pub struct SessionKeys {
    pub decrypt_key: Vec<u8>,
    pub recv_hmac_key: Vec<u8>,
    pub encrypt_key: Vec<u8>,
    pub send_hmac_key: Vec<u8>,
    pub pin_code: String,
}

pub fn gen_keypair() -> (SecretKey, PublicKey) {
    let secret_key = SecretKey::generate();
    let public_key = secret_key.public_key();

    (secret_key, public_key)
}

/// Returns the (x, y) coordinates of `key` encoded as signed big-endian integers,
/// which is what Java's `BigInteger` on the Android side expects.
pub fn encode_public_key(key: &PublicKey) -> (Vec<u8>, Vec<u8>) {
    let point = key.to_sec1_point(false);
    let x = point.x().expect("uncompressed point has x");
    let y = point.y().expect("uncompressed point has y");

    (encode_signed(x), encode_signed(y))
}

/// Parses a P-256 public key from signed big-endian coordinates. Those may carry a
/// leading sign byte or be shorter than 32 bytes when the value has leading zeros.
pub fn decode_public_key(x: &[u8], y: &[u8]) -> Result<PublicKey, anyhow::Error> {
    let mut bytes = [0u8; 65];
    bytes[0] = 0x04;
    copy_unsigned(x, &mut bytes[1..33])?;
    copy_unsigned(y, &mut bytes[33..65])?;

    let point = Sec1Point::from_bytes(bytes)?;
    Option::from(PublicKey::from_sec1_point(&point))
        .ok_or_else(|| anyhow!("UKey2: peer public key is not on the curve"))
}

pub fn derive_session_keys(
    private_key: &SecretKey,
    peer_key: &PublicKey,
    client_init: &[u8],
    server_init: &[u8],
    role: Role,
) -> Result<SessionKeys, anyhow::Error> {
    let dhs = diffie_hellman(private_key.to_nonzero_scalar(), peer_key.as_affine());
    let derived_secret = Sha256::digest(dhs.raw_secret_bytes());

    let ukey_info = [client_init, server_init].concat();
    let auth_string = hkdf(b"UKEY2 v1 auth", &derived_secret, &ukey_info)?;
    let next_secret = hkdf(b"UKEY2 v1 next", &derived_secret, &ukey_info)?;

    let d2d_client = hkdf(&D2D_SALT, &next_secret, b"client")?;
    let d2d_server = hkdf(&D2D_SALT, &next_secret, b"server")?;

    let client_key = hkdf(&SECURE_MESSAGE_SALT, &d2d_client, b"ENC:2")?;
    let client_hmac_key = hkdf(&SECURE_MESSAGE_SALT, &d2d_client, b"SIG:1")?;
    let server_key = hkdf(&SECURE_MESSAGE_SALT, &d2d_server, b"ENC:2")?;
    let server_hmac_key = hkdf(&SECURE_MESSAGE_SALT, &d2d_server, b"SIG:1")?;

    let pin_code = to_four_digit_string(&auth_string);

    Ok(match role {
        Role::Client => SessionKeys {
            decrypt_key: server_key,
            recv_hmac_key: server_hmac_key,
            encrypt_key: client_key,
            send_hmac_key: client_hmac_key,
            pin_code,
        },
        Role::Server => SessionKeys {
            decrypt_key: client_key,
            recv_hmac_key: client_hmac_key,
            encrypt_key: server_key,
            send_hmac_key: server_hmac_key,
            pin_code,
        },
    })
}

pub fn encrypt(key: &[u8], iv: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
    let cipher = cbc::Encryptor::<Aes256>::new_from_slices(key, iv)?;
    Ok(cipher.encrypt_padded_vec::<Pkcs7>(plaintext))
}

pub fn decrypt(key: &[u8], iv: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
    let cipher = cbc::Decryptor::<Aes256>::new_from_slices(key, iv)?;
    cipher
        .decrypt_padded_vec::<Pkcs7>(ciphertext)
        .map_err(|_| anyhow!("SecureMessage: invalid padding"))
}

pub fn sign(key: &[u8], data: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
    let mut mac = HmacSha256::new_from_slice(key)?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

pub fn verify(key: &[u8], data: &[u8], signature: &[u8]) -> Result<(), anyhow::Error> {
    let mut mac = HmacSha256::new_from_slice(key)?;
    mac.update(data);
    mac.verify_slice(signature)
        .map_err(|_| anyhow!("SecureMessage: hmac != signature"))
}

fn hkdf(salt: &[u8], input: &[u8], info: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
    let mut okm = vec![0u8; 32];
    Hkdf::<Sha256>::new(Some(salt), input)
        .expand(info, &mut okm)
        .map_err(|e| anyhow!("HKDF expand failed: {e}"))?;
    Ok(okm)
}

fn encode_signed(unsigned: &[u8]) -> Vec<u8> {
    let start = unsigned
        .iter()
        .position(|&b| b != 0)
        .unwrap_or(unsigned.len());
    let trimmed = &unsigned[start..];

    match trimmed.first() {
        None => vec![0],
        Some(b) if b & 0x80 != 0 => [&[0], trimmed].concat(),
        Some(_) => trimmed.to_vec(),
    }
}

fn copy_unsigned(signed: &[u8], dst: &mut [u8]) -> Result<(), anyhow::Error> {
    let start = signed.iter().position(|&b| b != 0).unwrap_or(signed.len());
    let trimmed = &signed[start..];
    if trimmed.len() > dst.len() {
        bail!("UKey2: public key coordinate is {} bytes", trimmed.len());
    }

    let offset = dst.len() - trimmed.len();
    dst[..offset].fill(0);
    dst[offset..].copy_from_slice(trimmed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_key_roundtrip() {
        for _ in 0..64 {
            let (_, public_key) = gen_keypair();
            let (x, y) = encode_public_key(&public_key);
            assert_eq!(decode_public_key(&x, &y).unwrap(), public_key);
        }
    }

    #[test]
    fn short_coordinates_are_left_padded() {
        let mut dst = [0xFFu8; 4];
        copy_unsigned(&[0x00, 0x01, 0x02], &mut dst).unwrap();
        assert_eq!(dst, [0x00, 0x00, 0x01, 0x02]);
    }

    #[test]
    fn signed_encoding_adds_sign_byte() {
        assert_eq!(encode_signed(&[0x00, 0x80, 0x01]), vec![0x00, 0x80, 0x01]);
        assert_eq!(encode_signed(&[0x00, 0x7F, 0x01]), vec![0x7F, 0x01]);
    }

    #[test]
    fn both_roles_agree() {
        let (client_priv, client_pub) = gen_keypair();
        let (server_priv, server_pub) = gen_keypair();

        let client =
            derive_session_keys(&client_priv, &server_pub, b"ci", b"si", Role::Client).unwrap();
        let server =
            derive_session_keys(&server_priv, &client_pub, b"ci", b"si", Role::Server).unwrap();

        assert_eq!(client.encrypt_key, server.decrypt_key);
        assert_eq!(client.send_hmac_key, server.recv_hmac_key);
        assert_eq!(client.pin_code, server.pin_code);

        let iv = [7u8; 16];
        let ct = encrypt(&client.encrypt_key, &iv, b"hello quick share").unwrap();
        assert_eq!(
            decrypt(&server.decrypt_key, &iv, &ct).unwrap(),
            b"hello quick share"
        );

        let sig = sign(&client.send_hmac_key, &ct).unwrap();
        verify(&server.recv_hmac_key, &ct, &sig).unwrap();
        assert!(verify(&server.recv_hmac_key, b"tampered", &sig).is_err());
    }
}
