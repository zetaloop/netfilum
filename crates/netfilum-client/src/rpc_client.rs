use netfilum::protocol::{Request, Response, RpcResult};
use netfilum::rpc::{read_message, write_message};
use std::io;
use std::net::{SocketAddr, TcpStream};
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

        write_message(&mut stream, request)?;
        let response: RpcResult<Response> = read_message(&mut stream)?;
        response.map_err(|error| error.to_io_error())
    }
}
