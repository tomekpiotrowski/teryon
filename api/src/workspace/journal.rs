//! Appends entries to a thread, keeping them in order.
//!
//! Entries are numbered within their thread so the application can show them
//! in the order they happened even when they arrive out of order.
//!
//! Only one writer appends to a thread at a time: the handler stores the
//! user's message before the agent starts, and after that the agent's turn is
//! the only writer until it ends. That is why reading the last position once
//! is enough.

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    Set,
};
use uuid::Uuid;

use crate::agents::EntryDraft;
use crate::workspace::{thread, thread_entry};

pub struct Journal {
    db: DatabaseConnection,
    thread_id: Uuid,
    user_id: Uuid,
    last_position: i64,
}

impl Journal {
    /// Opens the thread's journal after its last entry.
    pub async fn open(db: &DatabaseConnection, thread: &thread::Model) -> Result<Self, DbErr> {
        let last = thread_entry::Entity::find()
            .filter(thread_entry::Column::ThreadId.eq(thread.id))
            .order_by_desc(thread_entry::Column::Position)
            .one(db)
            .await?;

        Ok(Self {
            db: db.clone(),
            thread_id: thread.id,
            user_id: thread.user_id,
            last_position: last.map_or(0, |entry| entry.position),
        })
    }

    pub async fn add(&mut self, draft: EntryDraft) -> Result<thread_entry::Model, DbErr> {
        let entry = thread_entry::ActiveModel {
            id: Set(Uuid::new_v4()),
            user_id: Set(self.user_id),
            thread_id: Set(self.thread_id),
            position: Set(self.last_position + 1),
            kind: Set(draft.kind.name().to_string()),
            body: Set(draft.body),
            created_at: Set(Utc::now().naive_utc()),
            ..Default::default()
        }
        .insert(&self.db)
        .await?;

        self.last_position = entry.position;
        Ok(entry)
    }
}
