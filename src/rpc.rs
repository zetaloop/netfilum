use crate::protocol::RpcError;
use std::io::{self, Read, Write};

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
