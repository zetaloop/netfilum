#[cfg(not(unix))]
compile_error!("netfilum-server can only be built on Unix");

mod path;
mod server;

use clap::Parser;
use std::net::SocketAddr;

const DEFAULT_ADDR: &str = "127.0.0.1:4040";
const DEFAULT_VOLUME_LABEL: &str = "netfilum";
const DEFAULT_WSL_ROOT: &str = "/home/$USER/netfilum-root";

#[derive(Debug, Clone, Parser)]
#[command(name = "netfilumd")]
#[command(about = "A coursework RPC network file system daemon")]
struct ServerArgs {
    #[arg(long, default_value = DEFAULT_WSL_ROOT)]
    root: String,
    #[arg(long, default_value = DEFAULT_ADDR)]
    addr: SocketAddr,
    #[arg(long, default_value = DEFAULT_VOLUME_LABEL)]
    volume_label: String,
}

fn main() {
    if let Err(error) = run() {
        netfilum::print_error("error", format_args!("{error}"));
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = ServerArgs::parse();
    server::run(args.root, args.addr, args.volume_label)
}
