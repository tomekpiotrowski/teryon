//! The `teryon` program.

use std::env::current_exe;
use std::process::ExitCode;

use clap::Parser;

use teryon::command::{run, Arguments, Command, Context};
use teryon::logging;
use teryon::service::{Service, ServiceError};
use teryon::stopping::{stop_service, Stopped};
use teryon_core::paths::Folders;

#[tokio::main]
async fn main() -> ExitCode {
    let arguments = Arguments::parse();
    let folders = match &arguments.home {
        Some(home) => Folders::inside(home),
        None => match Folders::usual() {
            Ok(folders) => folders,
            Err(problem) => {
                eprintln!("{problem}");
                return ExitCode::FAILURE;
            }
        },
    };
    match arguments.command {
        Some(Command::Service { print_address }) => serve(folders, print_address).await,
        Some(Command::Stop) => stop(folders).await,
        Some(command) => {
            run(
                command,
                Context {
                    home: arguments.home,
                    folders,
                    json: arguments.json,
                },
            )
            .await
        }
        None => {
            eprintln!(
                "The terminal app is not there yet. Run `teryon service`, or `teryon --help`."
            );
            ExitCode::FAILURE
        }
    }
}

async fn serve(folders: Folders, print_address: Option<String>) -> ExitCode {
    let _log = logging::start(&folders.logs);
    let program = match current_exe() {
        Ok(program) => program,
        Err(problem) => {
            eprintln!("Could not tell where the teryon program is: {problem}");
            return ExitCode::FAILURE;
        }
    };
    let service = match Service::start(folders, program).await {
        Ok(service) => service,
        Err(ServiceError::AlreadyRunning) => {
            println!("Teryon is already running.");
            return ExitCode::SUCCESS;
        }
        Err(problem) => {
            eprintln!("{problem}");
            return ExitCode::FAILURE;
        }
    };
    if let Some(app) = print_address {
        let file = service.file();
        println!("{app}#port={}&token={}", file.port, file.token);
    }
    service.run(interrupted()).await;
    ExitCode::SUCCESS
}

async fn stop(folders: Folders) -> ExitCode {
    match stop_service(&folders.data).await {
        Ok(Stopped::Stopped) => {
            println!("Teryon has stopped.");
            ExitCode::SUCCESS
        }
        Ok(Stopped::NotRunning) => {
            println!("Teryon is not running.");
            ExitCode::SUCCESS
        }
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}

/// Resolves when the person presses Ctrl+C, or the system asks the program to end.
async fn interrupted() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut terminate =
            signal(SignalKind::terminate()).expect("the termination signal can be watched");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
