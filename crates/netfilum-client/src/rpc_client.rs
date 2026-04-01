use netfilum::protocol::{Request, Response, RpcResult};
use netfilum::rpc::{read_message, write_message, CONNECTION_DATA, CONNECTION_MONITOR};
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RpcClient {
    addr: SocketAddr,
    timeout: Duration,
}

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

        stream.write_all(&[CONNECTION_DATA])?;
        write_message(&mut stream, request)?;
        let response: RpcResult<Response> = read_message(&mut stream)?;
        response.map_err(|error| error.to_io_error())
    }
}

#[derive(Debug)]
pub struct MonitorConnection {
    stream: TcpStream,
}

impl MonitorConnection {
    pub fn connect(addr: SocketAddr) -> io::Result<Self> {
        let mut stream = TcpStream::connect(addr)?;
        stream.write_all(&[CONNECTION_MONITOR])?;
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(None)?;
        Ok(Self { stream })
    }

    pub fn wait_for_disconnect(&mut self) -> io::Result<()> {
        let mut buf = [0u8; 1];
        loop {
            match self.stream.read(&mut buf) {
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
