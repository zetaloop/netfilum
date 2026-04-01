fn main() {
    #[cfg(all(windows, target_env = "msvc"))]
    winfsp::build::winfsp_link_delayload();
}
