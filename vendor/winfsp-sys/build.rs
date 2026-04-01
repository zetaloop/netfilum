use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(feature = "system")]
use windows_registry::LOCAL_MACHINE;

static HEADER: &str = r#"
#include <winfsp/winfsp.h>
#include <winfsp/fsctl.h>
#include <winfsp/launch.h>
"#;

#[cfg(feature = "system")]
fn system() -> String {
    if !cfg!(windows) {
        panic!("'system' feature not supported for cross-platform compilation.");
    }

    let directory = LOCAL_MACHINE
        .open("SOFTWARE\\WOW6432Node\\WinFsp")
        .ok()
        .and_then(|u| u.get_string("InstallDir").ok())
        .expect("WinFsp installation directory not found.");

    println!("cargo:rustc-link-search={directory}/lib");
    format!("--include-directory={directory}/inc")
}

#[cfg(not(feature = "system"))]
fn local() -> String {
    let project_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing manifest dir"));

    println!(
        "cargo:rustc-link-search={}",
        project_dir.join("winfsp/lib").to_string_lossy()
    );

    "--include-directory=winfsp/inc".into()
}

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing OUT_DIR"));

    if cfg!(feature = "docsrs") {
        println!("cargo:warning=WinFSP docsrs stub build");
        File::create(out_dir.join("bindings.rs")).expect("failed to create docsrs bindings");
        return;
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_else(|_| "unknown".to_string());
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "unknown".to_string());
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_else(|_| "unknown".to_string());

    if target_os != "windows" {
        panic!("WinFSP is only supported on Windows.");
    }

    #[cfg(feature = "system")]
    let link_include = system();
    #[cfg(not(feature = "system"))]
    let link_include = local();

    println!("cargo:rustc-link-lib=dylib=delayimp");

    let (winfsp_lib, clang_target) = match (target_arch.as_str(), target_env.as_str()) {
        ("x86_64", "msvc") => ("winfsp-x64", "x86_64-pc-windows-msvc"),
        ("x86", "msvc") => ("winfsp-x86", "x86-pc-windows-msvc"),
        ("aarch64", "msvc") => ("winfsp-a64", "aarch64-pc-windows-msvc"),
        _ => panic!("unsupported triple {}", env::var("TARGET").expect("missing TARGET")),
    };

    println!("cargo:rustc-link-lib=dylib={winfsp_lib}");
    println!("cargo:rustc-link-arg=/DELAYLOAD:{winfsp_lib}.dll");

    let bindings_path = out_dir.join("bindings.rs");
    let bundled_bindings = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("missing manifest dir"))
        .join("src")
        .join("bindings.rs");

    if bundled_bindings.exists() {
        fs::copy(&bundled_bindings, &bindings_path).expect("failed to copy bundled bindings");
        return;
    }

    if !Path::new(&bindings_path).exists() {
        let gen_h_path = out_dir.join("gen.h");
        let mut gen_h = File::create(&gen_h_path).expect("could not create file");
        gen_h
            .write_all(HEADER.as_bytes())
            .expect("could not write header file");

        let bindings = bindgen::Builder::default()
            .header(gen_h_path.to_str().expect("invalid header path"))
            .derive_default(true)
            .blocklist_type("_?P?IMAGE_TLS_DIRECTORY.*")
            .allowlist_function("Fsp.*")
            .allowlist_type("FSP.*")
            .allowlist_type("Fsp.*")
            .allowlist_var("FSP_.*")
            .allowlist_var("Fsp.*")
            .allowlist_var("CTL_CODE")
            .clang_arg("-DUNICODE")
            .clang_arg(link_include)
            .clang_arg(format!("--target={clang_target}"))
            .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
            .generate()
            .expect("Unable to generate bindings");

        bindings
            .write_to_file(&bindings_path)
            .expect("Couldn't write bindings");
    }
}
