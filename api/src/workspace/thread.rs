//! A thread: one conversation with an agent inside a project.

use chrono::NaiveDateTime;
use erno::sync::syncable::Syncable;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

use crate::workspace::policy::OwnerPolicy;

/// The thread is waiting for a message.
pub const IDLE: &str = "idle";
/// An agent program is running for this thread.
pub const RUNNING: &str = "running";

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "threads")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id: Uuid,
    pub project_id: Uuid,
    pub title: String,
    pub agent: String,
    /// The identifier the agent program knows this conversation by.
    pub conversation_id: Uuid,
    /// Whether the agent program has seen the conversation yet.
    ///
    /// The programs take a different option for starting a conversation than
    /// for continuing one, and refuse the wrong one.
    pub conversation_started: bool,
    pub status: String,
    /// Whether an agent stopped working here since the person last looked.
    ///
    /// Kept on the server so that a thread read on one device is not
    /// shown as waiting on another.
    pub unread: bool,
    /// The names of the slash commands the agent program last said it takes, as a list of texts.
    ///
    /// Empty until the program has run once for this thread.
    pub commands: Json,
    pub created_at: NaiveDateTime,
    pub sync_seq: i64,
    pub deleted_at: Option<NaiveDateTime>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl Syncable for Entity {
    type Policy = OwnerPolicy;

    fn entity_type() -> &'static str {
        "threads"
    }

    fn sync_seq_column() -> Column {
        Column::SyncSeq
    }

    fn sync_seq(model: &Model) -> i64 {
        model.sync_seq
    }
}
