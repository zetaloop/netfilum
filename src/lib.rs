pub mod protocol;
pub mod rpc;

use std::fmt;
use std::io::Write as _;

pub fn print_status(args: fmt::Arguments<'_>) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_fmt(args);
    let _ = stdout.write_all(b"\r\n");
    let _ = stdout.flush();
}
