fn main() {
    if let Err(error) = netfilum::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
