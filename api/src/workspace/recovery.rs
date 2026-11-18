//! Puts threads right after Teryon stopped without warning.
//!
//! A thread is marked as running while its agent program runs. If Teryon is
//! killed, the programs die with it but the marks stay, and a thread marked as
//! running refuses every further message. Nothing can still be running when
//! Teryon starts, so every such thread is released then.

use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, IntoActiveModel,
    QueryFilter, Set,
};

use crate::agents::EntryDraft;
use crate::workspace::journal::Journal;
use crate::workspace::thread;

/// Marks every thread left as running as idle and unread, and says why in the thread.
pub async fn release_interrupted_threads(db: &DatabaseConnection) -> Result<usize, DbErr> {
    let threads = thread::Entity::find()
        .filter(thread::Column::Status.eq(thread::RUNNING))
        .all(db)
        .await?;
    let released = threads.len();

    for thread in threads {
        Journal::open(db, &thread)
            .await?
            .add(EntryDraft::notice("Interrupted by a restart."))
            .await?;
        let mut thread = thread.into_active_model();
        thread.status = Set(thread::IDLE.to_string());
        thread.unread = Set(true);
        thread.update(db).await?;
    }
    Ok(released)
}
