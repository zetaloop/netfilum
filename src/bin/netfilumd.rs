fn main() {
    if let Err(error) = netfilum::run_daemon() {
        netfilum::print_error(format_args!("{error}"));
        std::process::exit(1);
    }
}
