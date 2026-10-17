//! Native browser acceptance entry point, excluded from shipped builds.
fn main() {
    std::process::exit(colossus_desktop_lib::run_browser_acceptance());
}
