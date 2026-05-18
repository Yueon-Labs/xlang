fn main() {
    if let Err(err) = xlang::cli::run_cli() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}
