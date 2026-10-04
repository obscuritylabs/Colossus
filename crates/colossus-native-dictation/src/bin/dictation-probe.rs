fn main() {
    if let Err(error) = colossus_native_dictation::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
