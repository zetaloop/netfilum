mod path;
mod protocol;
mod rpc;
mod server;

#[cfg(windows)]
mod windows_mount;

use clap::{Parser, Subcommand};
use std::net::SocketAddr;

pub use path::{normalize_relative_path, windows_path_to_wsl};
pub use protocol::{DirEntry, EntryKind, ErrorCode, FileAttr, FileTimeValue, Request, Response};

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
#[command(name = "netfilumd")]
#[command(about = "A coursework RPC network file system daemon")]
pub struct ServerArgs {
    #[arg(long, default_value = DEFAULT_WSL_ROOT)]
    pub root: String,
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

pub fn run_client() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match Cli::parse().command {
        Command::Up(args) => run_up(args),
        Command::Mount(args) => run_mount(args),
    }
}

pub fn run_daemon() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    server::run(ServerArgs::parse())
}

fn run_up(_args: UpArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(windows)]
    {
        windows_mount::run_up(_args)
    }

    #[cfg(not(windows))]
    {
        let _ = _args;
        Err("`netfilum up` is only available on Windows".into())
    }
}

fn run_mount(_args: MountArgs) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(windows)]
    {
        windows_mount::run_mount(_args)
    }

    #[cfg(not(windows))]
    {
        let _ = _args;
        Err("`netfilum mount` is only available on Windows".into())
    }
}
