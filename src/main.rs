fn main() {
    if let Err(error) = netfilum::run_client() {
        netfilum::print_error(format_args!("{error}"));
        std::process::exit(1);
    }
}
