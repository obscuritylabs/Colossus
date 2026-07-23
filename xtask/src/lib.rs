//! Repository-local development and continuous-integration tasks.

mod checks;
mod cli;
mod command;
mod repository;
mod selection;

use cli::Invocation;
use repository::Repository;

/// Parse and run one repository task.
pub fn run(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let invocation = cli::parse(args)?;
    if invocation == Invocation::Help {
        print!("{}", cli::USAGE);
        return Ok(());
    }

    let repository = Repository::discover()?;
    checks::run(&repository, invocation)
}
