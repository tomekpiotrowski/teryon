//! The `teryon` program.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand};
use tokio::time::sleep;

use teryon::client::{Client, ClientError};
use teryon::logging;
use teryon::service::{Service, ServiceError};
use teryon::service_file::ServiceFile;
use teryon_core::paths::Folders;
use teryon_core::protocol::Request;

/// Teryon: agents working on your projects.
#[derive(Parser)]
#[command(name = "teryon", version)]
struct Arguments {
    /// Keep everything in this folder instead of the usual places.
    #[arg(long, global = true, env = "TERYON_HOME")]
    home: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the service that owns this machine's projects.
    Service {
        /// Print the address of the app at this address, with what it needs to connect.
        #[arg(long, value_name = "APP ADDRESS")]
        print_address: Option<String>,
    },
    /// Stop the running service, after stopping every agent.
    Stop,
}

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
    let service = match Service::start(folders).await {
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
    let file = match ServiceFile::read(&folders.data) {
        Ok(Some(file)) => file,
        Ok(None) => {
            println!("Teryon is not running.");
            return ExitCode::SUCCESS;
        }
        Err(problem) => {
            eprintln!("{problem}");
            return ExitCode::FAILURE;
        }
    };
    let mut client = match Client::connect_to(&file).await {
        Ok(client) => client,
        Err(ClientError::Unreachable(_) | ClientError::Closed) => {
            println!("Teryon is not running.");
            return ExitCode::SUCCESS;
        }
        Err(problem) => {
            eprintln!("{problem}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(problem) = client.call(Request::Shutdown).await {
        eprintln!("{problem}");
        return ExitCode::FAILURE;
    }
    // The service removes its file once every agent has stopped.
    for _ in 0..300 {
        if !matches!(ServiceFile::read(&folders.data), Ok(Some(current)) if current == file) {
            println!("Teryon has stopped.");
            return ExitCode::SUCCESS;
        }
        sleep(Duration::from_millis(100)).await;
    }
    eprintln!("Teryon is taking long to stop.");
    ExitCode::FAILURE
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
