use crate::protocol::{Request, Response, RpcError, RpcResult};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct RpcClient {
    addr: SocketAddr,
    timeout: Duration,
}

#[allow(dead_code)]
impl RpcClient {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            timeout: Duration::from_secs(5),
        }
    }

    pub fn send(&self, request: &Request) -> io::Result<Response> {
        let mut stream = TcpStream::connect(self.addr)?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;

        write_message(&mut stream, request)?;
        let response: RpcResult<Response> = read_message(&mut stream)?;
        response.map_err(|error| error.to_io_error())
    }
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

#[allow(dead_code)]
pub fn ready(addr: SocketAddr, timeout: Duration) -> bool {
    TcpStream::connect_timeout(&addr, timeout).is_ok()
}

impl From<RpcError> for io::Error {
    fn from(value: RpcError) -> Self {
        value.to_io_error()
    }
}
