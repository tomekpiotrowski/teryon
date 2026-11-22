//! Runs the local service until it is told to stop.
//!
//! One service per person per machine. A lock on `service.lock` in the data
//! folder makes sure of that: a second service sees the lock taken and ends.
//! The service opens the workspace, listens, writes `service.json` once it is
//! listening, and on the way out stops every agent and removes the file.

use std::fs::{create_dir_all, File, OpenOptions, TryLockError};
use std::path::PathBuf;
use std::process::id;
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;
use tokio::sync::watch;
use tracing::info;

use teryon_core::paths::Folders;
use teryon_core::protocol::fingerprint;
use teryon_core::settings::{Settings, SettingsError};
use teryon_core::workflow::Workflows;
use teryon_core::workspace::{AgentTools, Setup, Workspace, WorkspaceError};

use crate::local::{bind, router, Listener};
use crate::service_file::{ServiceFile, ServiceFileError};

/// How long running agents get to stop when the service stops.
const STOP_PATIENCE: Duration = Duration::from_secs(10);

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("Teryon is already running.")]
    AlreadyRunning,
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    ServiceFile(#[from] ServiceFileError),
    #[error("Could not listen for connections: {0}")]
    Listen(std::io::Error),
    #[error("Could not use the data folder: {0}")]
    DataFolder(std::io::Error),
}

/// A service that is listening, before it runs.
pub struct Service {
    folders: Folders,
    file: ServiceFile,
    listener: tokio::net::TcpListener,
    state: Arc<Listener>,
    stopping: watch::Receiver<bool>,
    _lock: File,
}

impl Service {
    /// Takes the lock, opens the workspace, listens and writes `service.json`.
    ///
    /// `program` is the `teryon` program agents call to start a change: the
    /// one running this service, in practice.
    pub async fn start(folders: Folders, program: PathBuf) -> Result<Self, ServiceError> {
        create_dir_all(&folders.data).map_err(ServiceError::DataFolder)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(folders.data.join("service.lock"))
            .map_err(ServiceError::DataFolder)?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(ServiceError::AlreadyRunning),
            Err(TryLockError::Error(problem)) => return Err(ServiceError::DataFolder(problem)),
        }

        let settings = Settings::read(&folders.config)?;
        let workspace = Workspace::open(
            &folders.data,
            Setup {
                agents: settings.agents.clone(),
                open: settings.open.clone(),
                github: settings.github.clone(),
                tools: Some(AgentTools {
                    program,
                    data: folders.data.clone(),
                }),
                workflow: settings.workflow.clone(),
                stages: settings.stages.clone(),
                workflows: Workflows::from_settings(&settings),
                own_workflows: settings.workflows.is_some(),
                config_folder: Some(folders.config.clone()),
            },
        )?;
        let listener = bind(settings.service.port)
            .await
            .map_err(ServiceError::Listen)?;
        let port = listener.local_addr().map_err(ServiceError::Listen)?.port();
        let (stopping_sender, stopping) = watch::channel(false);
        let file = ServiceFile {
            port,
            token: new_token(),
            process: id(),
            protocol: fingerprint().to_string(),
        };
        let state = Arc::new(Listener {
            workspace,
            token: file.token.clone(),
            port,
            allowed_origins: settings.service.allowed_origins,
            stopping: stopping_sender,
        });
        file.write(&folders.data)?;
        info!("listening on 127.0.0.1:{port}");
        Ok(Self {
            folders,
            file,
            listener,
            state,
            stopping,
            _lock: lock,
        })
    }

    pub fn file(&self) -> &ServiceFile {
        &self.file
    }

    /// Serves until a client asks the service to stop or `interrupted` resolves.
    pub async fn run(self, interrupted: impl std::future::Future<Output = ()> + Send + 'static) {
        let Self {
            folders,
            file,
            listener,
            state,
            mut stopping,
            _lock,
        } = self;
        let workspace = state.workspace.clone();
        let listing = workspace.clone();
        tokio::spawn(async move { listing.list_models().await });
        let asked = async move {
            let _ = stopping.wait_for(|stop| *stop).await;
        };
        let server = axum::serve(listener, router(state)).with_graceful_shutdown(async move {
            tokio::select! {
                () = asked => {}
                () = interrupted => {}
            }
        });
        if let Err(problem) = server.await {
            tracing::error!("the service stopped with a problem: {problem}");
        }
        info!("stopping agents");
        workspace.shut_down(STOP_PATIENCE).await;
        file.remove_if_mine(&folders.data);
        info!("stopped");
    }
}

/// A fresh secret: 32 random bytes as hexadecimal digits.
fn new_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("the system can give random bytes");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
