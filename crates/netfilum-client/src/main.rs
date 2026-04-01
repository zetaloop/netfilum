#[cfg(not(windows))]
compile_error!("netfilum-client can only be built on Windows");

mod mount;
mod path;
mod rpc_client;

use clap::{Parser, Subcommand};
use std::net::SocketAddr;

const DEFAULT_ADDR: &str = "127.0.0.1:4040";
const DEFAULT_MOUNT: &str = "N:";
const DEFAULT_VOLUME_LABEL: &str = "netfilum";
const DEFAULT_WSL_DISTRO: &str = "Ubuntu";
const DEFAULT_WSL_ROOT: &str = "/home/$USER/netfilum-root";

#[derive(Debug, Parser)]
#[command(name = "netfilum")]
#[command(about = "A coursework RPC network file system client")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Up(UpArgs),
    Mount(MountArgs),
}

#[derive(Debug, Clone, Parser)]
pub struct UpArgs {
    #[arg(long, default_value = DEFAULT_WSL_DISTRO)]
    pub distro: String,
    #[arg(long, default_value = DEFAULT_WSL_ROOT)]
    pub root: String,
    #[arg(long, default_value = DEFAULT_MOUNT)]
    pub mount: String,
    #[arg(long, default_value = DEFAULT_ADDR)]
    pub addr: SocketAddr,
    #[arg(long, default_value = DEFAULT_VOLUME_LABEL)]
    pub volume_label: String,
}

#[derive(Debug, Clone, Parser)]
pub struct MountArgs {
    #[arg(long, default_value = DEFAULT_MOUNT)]
    pub mount: String,
    #[arg(long, default_value = DEFAULT_ADDR)]
    pub addr: SocketAddr,
    #[arg(long, default_value = DEFAULT_VOLUME_LABEL)]
    pub volume_label: String,
}

fn main() {
    if let Err(error) = run() {
        netfilum::print_error("error", format_args!("{error}"));
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match Cli::parse().command {
        Command::Up(args) => mount::run_up(args),
        Command::Mount(args) => mount::run_mount(args),
    }
}
