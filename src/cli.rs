use std::io::{IsTerminal, Write};
use std::sync::OnceLock;

static USE_COLOR: OnceLock<bool> = OnceLock::new();

fn use_color() -> bool {
    *USE_COLOR.get_or_init(|| std::io::stderr().is_terminal())
}

const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

pub fn highlight(value: impl std::fmt::Display) -> String {
    if use_color() {
        format!("{BOLD}{CYAN}{value}{RESET}")
    } else {
        value.to_string()
    }
}

pub fn print_info(label: &str, message: std::fmt::Arguments<'_>) {
    let mut out = std::io::stderr().lock();
    if use_color() {
        let _ = write!(out, "{BOLD}{GREEN}{label}{RESET} {message}\r\n");
    } else {
        let _ = write!(out, "{label} {message}\r\n");
    }
    let _ = out.flush();
}

pub fn print_warn(label: &str, message: std::fmt::Arguments<'_>) {
    let mut out = std::io::stderr().lock();
    if use_color() {
        let _ = write!(out, "{BOLD}{YELLOW}{label}{RESET} {message}\r\n");
    } else {
        let _ = write!(out, "{label} {message}\r\n");
    }
    let _ = out.flush();
}

pub fn print_error(label: &str, message: std::fmt::Arguments<'_>) {
    let mut out = std::io::stderr().lock();
    if use_color() {
        let _ = write!(out, "{BOLD}{RED}{label}{RESET} {message}\r\n");
    } else {
        let _ = write!(out, "{label} {message}\r\n");
    }
    let _ = out.flush();
}
