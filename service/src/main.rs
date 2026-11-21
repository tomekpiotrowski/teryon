//! The `teryon` program.

use std::env::{current_exe, var};
use std::io::{stdin, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use teryon::client::Client;
use teryon::logging;
use teryon::service::{Service, ServiceError};
use teryon::service_file::ServiceFile;
use teryon::stopping::{stop_service, Stopped};
use teryon_core::agents::asking::ASKED;
use teryon_core::paths::Folders;
use teryon_core::protocol::{Answer, Request};
use uuid::Uuid;

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
    /// For agents working in Teryon: their own copy of the project to change files in.
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommand,
    },
    /// For agents working in Teryon: the thread they work in.
    Thread {
        #[command(subcommand)]
        command: ThreadCommand,
    },
}

#[derive(Subcommand)]
enum ThreadCommand {
    /// Give this thread a title of a few words, shown in place of the start of its first message.
    Title {
        /// The title, such as `Fix the login redirect`.
        title: String,
    },
    /// Ask the person questions, written as JSON on standard input; their answers arrive as their next message.
    Ask,
}

#[derive(Subcommand)]
enum WorktreeCommand {
    /// Create a git worktree on a new branch for this thread, and print its path.
    Create {
        /// The name of the worktree and of its branch, such as `fix-login-redirect`.
        name: String,
        /// The branch to start from. Without it, the branch the project is on.
        #[arg(long, value_name = "BRANCH")]
        from: Option<String>,
    },
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
        Some(Command::Worktree {
            command: WorktreeCommand::Create { name, from },
        }) => create_worktree(folders, name, from).await,
        Some(Command::Thread {
            command: ThreadCommand::Title { title },
        }) => title_thread(folders, title).await,
        Some(Command::Thread {
            command: ThreadCommand::Ask,
        }) => ask_questions(folders).await,
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

/// Asks the service for a worktree for the thread whose agent runs this, and prints its path.
async fn create_worktree(folders: Folders, name: String, from: Option<String>) -> ExitCode {
    let answer = call_for_thread(folders, "teryon worktree create", |thread_id, turn| {
        Request::StartChange {
            thread_id,
            turn,
            name,
            from,
        }
    })
    .await;
    match answer {
        Ok(Answer::Path { path }) => {
            println!("{path}");
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("Teryon answered with something other than a path.");
            ExitCode::FAILURE
        }
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}

/// Gives the thread whose agent runs this a title.
async fn title_thread(folders: Folders, title: String) -> ExitCode {
    let answer = call_for_thread(folders, "teryon thread title", |thread_id, turn| {
        Request::NameThread {
            thread_id,
            turn,
            title,
        }
    })
    .await;
    match answer {
        Ok(_) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}

/// Asks the person the questions on standard input, for the thread whose agent runs this.
///
/// The service reads the JSON, so its log has every refusal; this only passes it on.
async fn ask_questions(folders: Folders) -> ExitCode {
    let mut asked = String::new();
    if let Err(problem) = stdin().read_to_string(&mut asked) {
        eprintln!("Could not read the questions from standard input: {problem}");
        return ExitCode::FAILURE;
    }
    let answer = call_for_thread(folders, "teryon thread ask", |thread_id, turn| {
        Request::AskQuestions {
            thread_id,
            turn,
            asked,
        }
    })
    .await;
    match answer {
        Ok(_) => {
            println!("{ASKED}");
            ExitCode::SUCCESS
        }
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}

/// Sends the service a request about the thread whose agent runs this, during its turn.
///
/// The service sets `TERYON_THREAD` and `TERYON_TURN` for the agents it
/// starts, and `TERYON_DATA` to the folder with its `service.json`. The
/// error is the text to print.
async fn call_for_thread(
    folders: Folders,
    command: &str,
    request: impl FnOnce(Uuid, Uuid) -> Request,
) -> Result<Answer, String> {
    let (Some(thread_id), Some(turn)) = (identifier("TERYON_THREAD"), identifier("TERYON_TURN"))
    else {
        return Err(format!("`{command}` is for agents working in Teryon, which sets TERYON_THREAD and TERYON_TURN for them."));
    };
    let data = var("TERYON_DATA").map_or(folders.data, PathBuf::from);
    let file = match ServiceFile::read(&data) {
        Ok(Some(file)) => file,
        Ok(None) => return Err("Teryon is not running.".to_string()),
        Err(problem) => return Err(problem.to_string()),
    };
    let mut client = Client::connect_to(&file)
        .await
        .map_err(|problem| problem.to_string())?;
    client
        .call(request(thread_id, turn))
        .await
        .map_err(|problem| problem.to_string())
}

/// An identifier from an environment variable, when it is set to one.
fn identifier(name: &str) -> Option<Uuid> {
    var(name)
        .ok()
        .and_then(|value| Uuid::parse_str(&value).ok())
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
