//! Command-line entry point for repository-local tasks.

fn main() {
    if let Err(error) = xtask::run(std::env::args().skip(1)) {
        eprintln!("xtask: {error}");
        std::process::exit(1);
    }
}
