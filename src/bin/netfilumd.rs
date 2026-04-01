fn main() {
    if let Err(error) = netfilum::run_daemon() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
