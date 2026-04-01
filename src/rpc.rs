use crate::protocol::RpcError;
#[cfg(windows)]
use crate::protocol::{Request, Response, RpcResult};
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{ChaCha20Poly1305, KeyInit, Nonce};
use ring::pbkdf2;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use socket2::SockRef;
use std::io::{self, Read, Write};
#[cfg(not(windows))]
use std::net::TcpStream;
use std::num::NonZeroU32;
#[cfg(windows)]
use std::net::{Shutdown, SocketAddr, TcpStream};
#[cfg(windows)]
use std::sync::{Arc, Mutex};
#[cfg(windows)]
use std::time::Duration;

pub(crate) const AUTH_TOKEN: [u8; 16] = *b"netfilum-auth-v1";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
const SALT_LEN: usize = 16;
const PBKDF2_ROUNDS: u32 = 100_000;

#[cfg(windows)]
#[derive(Debug, Clone)]
pub struct RpcClient {
    addr: SocketAddr,
    timeout: Duration,
    password: String,
    session: Arc<Mutex<Option<RpcSession>>>,
}

#[derive(Debug)]
pub(crate) enum RpcTransport {
    Plaintext(TcpStream),
    Encrypted {
        stream: TcpStream,
        key: [u8; KEY_LEN],
    },
}

#[cfg(windows)]
#[derive(Debug)]
struct RpcSession {
    transport: RpcTransport,
}

#[cfg(windows)]
#[derive(Debug)]
pub struct MonitorConnection {
    stream: TcpStream,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AuthRequest {
    pub token: [u8; 16],
    pub kind: ConnectionKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ServerHello {
    pub transport: TransportMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) enum TransportMode {
    Plaintext,
    Encrypted { salt: [u8; SALT_LEN] },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ConnectionKind {
    Data,
    Monitor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedMessage {
    nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
}

#[cfg(windows)]
impl RpcClient {
    pub fn new(addr: SocketAddr, password: String) -> Self {
        Self {
            addr,
            timeout: Duration::from_secs(5),
            password,
            session: Arc::new(Mutex::new(None)),
        }
    }

    pub fn send(&self, request: &Request) -> io::Result<Response> {
        let mut session = self.session.lock().expect("rpc session lock poisoned");
        if session.is_none() {
            *session = Some(RpcSession::connect(
                self.addr,
                self.timeout,
                &self.password,
            )?);
        }

        match session
            .as_mut()
            .expect("rpc session missing after initialization")
            .send(request)
        {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(error)) => Err(error.to_io_error()),
            Err(error) => {
                *session = None;
                Err(error)
            }
        }
    }
}

impl RpcTransport {
    pub(crate) fn send<T: serde::Serialize>(&mut self, value: &T) -> io::Result<()> {
        match self {
            Self::Plaintext(stream) => write_message(stream, value),
            Self::Encrypted { stream, key } => write_encrypted_message(stream, key, value),
        }
    }

    pub(crate) fn receive<T: DeserializeOwned>(&mut self) -> io::Result<T> {
        match self {
            Self::Plaintext(stream) => read_message(stream),
            Self::Encrypted { stream, key } => read_encrypted_message(stream, key),
        }
    }

    pub(crate) fn into_stream(self) -> TcpStream {
        match self {
            Self::Plaintext(stream) | Self::Encrypted { stream, .. } => stream,
        }
    }
}

#[cfg(windows)]
impl RpcSession {
    fn connect(addr: SocketAddr, timeout: Duration, password: &str) -> io::Result<Self> {
        let transport = connect_transport(addr, timeout, password, ConnectionKind::Data)?;
        Ok(Self { transport })
    }

    fn send(&mut self, request: &Request) -> io::Result<RpcResult<Response>> {
        self.transport.send(request)?;
        self.transport.receive()
    }
}

#[cfg(windows)]
impl MonitorConnection {
    pub fn connect(addr: SocketAddr, password: &str) -> io::Result<Self> {
        let transport = connect_transport(
            addr,
            Duration::from_secs(5),
            password,
            ConnectionKind::Monitor,
        )?;
        let stream = transport.into_stream();
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(None)?;
        SockRef::from(&stream).set_keepalive(true)?;
        Ok(Self { stream })
    }

    pub fn wait_for_disconnect(&mut self) -> io::Result<()> {
        let mut buffer = [0u8; 1];
        loop {
            match self.stream.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(_) => {}
                Err(error) => return Err(error),
            }
        }
    }

    pub fn shutdown(&self) -> io::Result<()> {
        self.stream.shutdown(Shutdown::Both)
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            stream: self.stream.try_clone()?,
        })
    }
}

#[cfg(windows)]
fn connect_transport(
    addr: SocketAddr,
    timeout: Duration,
    password: &str,
    kind: ConnectionKind,
) -> io::Result<RpcTransport> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let hello: ServerHello = read_message(&mut stream)?;
    let mut transport = match hello.transport {
        TransportMode::Plaintext => {
            if password.is_empty() {
                RpcTransport::Plaintext(stream)
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "server is using plaintext transport but client was configured with a password",
                ));
            }
        }
        TransportMode::Encrypted { salt } => {
            let key = derive_transport_key(password, &salt);
            RpcTransport::Encrypted { stream, key }
        }
    };

    transport.send(&AuthRequest {
        token: AUTH_TOKEN,
        kind,
    })?;
    let auth_result: RpcResult<()> = transport.receive()?;
    auth_result.map_err(|error| error.to_io_error())?;
    Ok(transport)
}

pub(crate) fn derive_transport_key(password: &str, salt: &[u8; SALT_LEN]) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(PBKDF2_ROUNDS).expect("PBKDF2 rounds must be non-zero"),
        salt,
        password.as_bytes(),
        &mut key,
    );
    key
}

pub(crate) fn random_bytes<const N: usize>() -> io::Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes)
}

pub(crate) fn write_encrypted_message<T: serde::Serialize>(
    writer: &mut impl Write,
    key: &[u8; KEY_LEN],
    value: &T,
) -> io::Result<()> {
    let plaintext =
        bincode::serde::encode_to_vec(value, bincode::config::standard()).map_err(encode_err)?;
    let nonce = random_bytes::<NONCE_LEN>()?;
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|error| io::Error::other(error.to_string()))?;
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|error| io::Error::other(error.to_string()))?;
    write_message(writer, &EncryptedMessage { nonce, ciphertext })
}

pub(crate) fn read_encrypted_message<T: serde::de::DeserializeOwned>(
    reader: &mut impl Read,
    key: &[u8; KEY_LEN],
) -> io::Result<T> {
    let packet: EncryptedMessage = read_message(reader)?;
    let cipher = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|error| io::Error::other(error.to_string()))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&packet.nonce), packet.ciphertext.as_ref())
        .map_err(|error| io::Error::new(io::ErrorKind::PermissionDenied, error.to_string()))?;
    let (value, _) = bincode::serde::decode_from_slice(&plaintext, bincode::config::standard())
        .map_err(decode_err)?;
    Ok(value)
}

pub fn write_message<T: serde::Serialize>(writer: &mut impl Write, value: &T) -> io::Result<()> {
    let bytes =
        bincode::serde::encode_to_vec(value, bincode::config::standard()).map_err(encode_err)?;
    let len = bytes.len() as u32;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

pub fn read_message<T: serde::de::DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload)?;
    let (value, _) = bincode::serde::decode_from_slice(&payload, bincode::config::standard())
        .map_err(decode_err)?;
    Ok(value)
}

fn encode_err(error: bincode::error::EncodeError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

fn decode_err(error: bincode::error::DecodeError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

impl From<RpcError> for io::Error {
    fn from(value: RpcError) -> Self {
        value.to_io_error()
    }
}
