use crate::protocol::RpcError;
use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use std::io::{self, Read, Write};

pub const CONNECTION_DATA: u8 = 0x00;
pub const CONNECTION_MONITOR: u8 = 0x01;
const NONCE_LEN: usize = 12;

#[derive(Debug, Clone)]
pub struct RpcCipher {
    key: [u8; 32],
}

impl RpcCipher {
    pub fn from_password(password: &str) -> io::Result<Self> {
        Ok(Self {
            key: derive_key(password)?,
        })
    }

    fn encrypt(&self, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(codec_err)?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher.encrypt(&nonce, plaintext).map_err(codec_err)?;
        let mut payload = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        payload.extend_from_slice(&nonce);
        payload.extend_from_slice(&ciphertext);
        Ok(payload)
    }

    fn decrypt(&self, payload: &[u8]) -> io::Result<Vec<u8>> {
        if payload.len() < NONCE_LEN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "encrypted frame is too short",
            ));
        }

        let cipher = Aes256Gcm::new_from_slice(&self.key).map_err(codec_err)?;
        let (nonce_bytes, ciphertext) = payload.split_at(NONCE_LEN);
        cipher
            .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
            .map_err(codec_err)
    }
}

pub fn write_message<T: serde::Serialize>(
    writer: &mut impl Write,
    cipher: &RpcCipher,
    value: &T,
) -> io::Result<()> {
    let bytes = postcard::to_allocvec(value).map_err(codec_err)?;
    let bytes = cipher.encrypt(&bytes)?;
    let len = u32::try_from(bytes.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "encrypted frame exceeds u32 length limit",
        )
    })?;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

pub fn read_message<T: serde::de::DeserializeOwned>(
    reader: &mut impl Read,
    cipher: &RpcCipher,
) -> io::Result<T> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    let payload = cipher.decrypt(&payload)?;
    postcard::from_bytes(&payload).map_err(codec_err)
}

fn derive_key(password: &str) -> io::Result<[u8; 32]> {
    let salt_hash = blake3::hash(password.as_bytes());
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&salt_hash.as_bytes()[..16]);

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default());
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(password.as_bytes(), &salt, &mut key)
        .map_err(codec_err)?;
    Ok(key)
}

fn codec_err(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

impl From<RpcError> for io::Error {
    fn from(value: RpcError) -> Self {
        value.to_io_error()
    }
}

#[cfg(test)]
mod tests {
    use super::{RpcCipher, read_message, write_message};
    use crate::protocol::Request;
    use std::io::{self, Cursor};

    #[test]
    fn roundtrips_encrypted_message() {
        let cipher = RpcCipher::from_password("secret").unwrap();
        let request = Request::Stat {
            path: "demo.txt".to_string(),
        };
        let mut encoded = Vec::new();

        write_message(&mut encoded, &cipher, &request).unwrap();

        let decoded: Request = read_message(&mut Cursor::new(encoded), &cipher).unwrap();
        assert_eq!(request, decoded);
    }

    #[test]
    fn rejects_message_with_wrong_password() {
        let write_cipher = RpcCipher::from_password("secret").unwrap();
        let read_cipher = RpcCipher::from_password("wrong").unwrap();
        let mut encoded = Vec::new();

        write_message(
            &mut encoded,
            &write_cipher,
            &Request::Open {
                path: "demo.txt".to_string(),
            },
        )
        .unwrap();

        let error = read_message::<Request>(&mut Cursor::new(encoded), &read_cipher).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
