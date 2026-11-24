//! Commands an agent runs on its own thread during a turn.
//!
//! These require `TERYON_THREAD` and `TERYON_TURN`, which the service sets for
//! every agent it starts, so a command copied to a terminal cannot accidentally
//! rename or move a thread outside a turn.

use std::env::var;
use std::io::{stdin, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use uuid::Uuid;

use teryon_core::agents::asking::ASKED;
use teryon_core::paths::Folders;
use teryon_core::protocol::{Answer, Request};

use crate::client::Client;
use crate::service_file::ServiceFile;

/// Asks the service for a worktree for the thread whose agent runs this, and prints its path.
pub async fn create_worktree(
    home: Option<&Path>,
    folders: Folders,
    name: String,
    from: Option<String>,
) -> ExitCode {
    let answer = call_for_thread(home, folders, "teryon worktree create", |thread_id, turn| {
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
pub async fn title_thread(home: Option<&Path>, folders: Folders, title: String) -> ExitCode {
    simple_call(home, folders, "teryon thread title", |thread_id, turn| {
        Request::NameThread {
            thread_id,
            turn,
            title,
        }
    })
    .await
}

/// Reports that the thread whose agent runs this has moved to another stage, and prints what the agent should do next.
pub async fn report_stage(home: Option<&Path>, folders: Folders, stage: String) -> ExitCode {
    let answer = call_for_thread(home, folders, "teryon thread stage", |thread_id, turn| {
        Request::ReportStage {
            thread_id,
            turn,
            stage,
        }
    })
    .await;
    match answer {
        Ok(Answer::Stage { text }) => {
            if let Some(text) = text {
                println!("{text}");
            }
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("Teryon answered with something other than a stage reply.");
            ExitCode::FAILURE
        }
        Err(problem) => {
            eprintln!("{problem}");
            ExitCode::FAILURE
        }
    }
}

/// Puts the thread whose agent runs this on another workflow.
pub async fn choose_own_workflow(home: Option<&Path>, folders: Folders, name: String) -> ExitCode {
    simple_call(home, folders, "teryon thread workflow", |thread_id, turn| {
        Request::ChooseWorkflow {
            thread_id,
            turn: Some(turn),
            workflow: name,
        }
    })
    .await
}

/// Asks the person the questions on standard input, for the thread whose agent runs this.
///
/// The service reads the JSON, so its log has every refusal; this only passes it on.
pub async fn ask_questions(home: Option<&Path>, folders: Folders) -> ExitCode {
    let mut asked = String::new();
    if let Err(problem) = stdin().read_to_string(&mut asked) {
        eprintln!("Could not read the questions from standard input: {problem}");
        return ExitCode::FAILURE;
    }
    let answer = call_for_thread(home, folders, "teryon thread ask", |thread_id, turn| {
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

/// Sends a request about the thread whose agent runs this, printing nothing unless it was refused.
async fn simple_call(
    home: Option<&Path>,
    folders: Folders,
    command: &str,
    request: impl FnOnce(Uuid, Uuid) -> Request,
) -> ExitCode {
    let answer = call_for_thread(home, folders, command, request).await;
    match answer {
        Ok(_) => ExitCode::SUCCESS,
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
    home: Option<&Path>,
    folders: Folders,
    command: &str,
    request: impl FnOnce(Uuid, Uuid) -> Request,
) -> Result<Answer, String> {
    let (Some(thread_id), Some(turn)) = (identifier("TERYON_THREAD"), identifier("TERYON_TURN"))
    else {
        return Err(format!(
            "`{command}` is for agents working in Teryon, which sets TERYON_THREAD and TERYON_TURN for them."
        ));
    };
    let data = data_folder(home, folders);
    let file = match ServiceFile::read(&data) {
        Ok(Some(file)) => file,
        Ok(None) => return Err("Teryon is not running.".to_string()),
        Err(problem) => return Err(problem.to_string()),
    };
    let mut client = Client::connect_as(&file, Some(thread_id))
        .await
        .map_err(|problem| problem.to_string())?;
    client
        .call(request(thread_id, turn))
        .await
        .map_err(|problem| problem.to_string())
}

/// Where `service.json` lives: explicit `--home`, else `TERYON_DATA`, else the usual data folder.
pub fn data_folder(home: Option<&Path>, folders: Folders) -> PathBuf {
    if home.is_some() {
        return folders.data;
    }
    var("TERYON_DATA").map_or(folders.data, PathBuf::from)
}

/// An identifier from an environment variable, when it is set to one.
pub fn identifier(name: &str) -> Option<Uuid> {
    var(name)
        .ok()
        .and_then(|value| Uuid::parse_str(&value).ok())
}
