//! Explicit macOS Debug process fixture, never a production browser launcher.
#![forbid(unsafe_code)]
#[cfg(not(all(target_os = "macos", debug_assertions)))]
compile_error!("the native TLS acceptance parent requires macOS Debug");

#[path = "native_parent_acceptance/arguments.rs"]
mod arguments;
#[path = "../src/entry_seal.rs"]
mod cdhash;
#[path = "native_parent_acceptance/child.rs"]
mod child;
#[path = "native_parent_acceptance/crash.rs"]
mod crash;
#[path = "native_parent_acceptance/evidence.rs"]
mod evidence;
#[path = "native_parent_acceptance/tls.rs"]
mod tls;
#[path = "native_parent_acceptance/wire.rs"]
mod wire;

use arguments::{Arguments, Mode};
use std::{io::Write as _, process::ExitCode};

type Failure = &'static str;

fn main() -> ExitCode {
    let args = match Arguments::parse(std::env::args_os().skip(1).collect()) {
        Ok(args) => args,
        Err(_) => return fail("invalid_native_fixture_arguments"),
    };
    // The crash-child consumes its fixed fd3 before runtime descriptors exist.
    let controller = if args.mode == Mode::CrashChild {
        match colossus_darwin_process::take_connected_parent_channel(args.parent_cdhash) {
            Ok(channel) => Some(channel),
            Err(_) => return fail("crash_controller_admission_rejected"),
        }
    } else {
        None
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return fail("fixture_runtime_unavailable"),
    };
    let result = runtime.block_on(async {
        match args.mode {
            Mode::ParentCrash => crash::controller(&args).await,
            Mode::CrashChild => {
                crash::sacrificial(&args, controller.ok_or("missing_crash_controller")?).await
            }
            _ => child::accept(&args).await,
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(category) => fail(category),
    }
}

fn fail(category: Failure) -> ExitCode {
    let _ = writeln!(
        std::io::stderr().lock(),
        "colossus_native_tls_acceptance: {category}"
    );
    ExitCode::from(1)
}
