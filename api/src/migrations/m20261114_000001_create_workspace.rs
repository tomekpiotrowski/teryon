//! Creates the tables behind the workspace: projects, threads and thread entries.
//!
//! All three are synchronised to the application through erno's sync columns,
//! so every row carries its owner: erno decides who receives a change by
//! looking at the row alone.

use erno::sync::migration::{add_sync_columns, remove_sync_columns};
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Projects {
    Table,
    Id,
    UserId,
    Name,
    Directory,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Threads {
    Table,
    Id,
    UserId,
    ProjectId,
    Title,
    Agent,
    ConversationId,
    ConversationStarted,
    Status,
    CreatedAt,
}

#[derive(DeriveIden)]
enum ThreadEntries {
    Table,
    Id,
    UserId,
    ThreadId,
    Position,
    Kind,
    Body,
    CreatedAt,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Projects::Table)
                    .col(ColumnDef::new(Projects::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Projects::UserId).uuid().not_null())
                    .col(ColumnDef::new(Projects::Name).text().not_null())
                    .col(ColumnDef::new(Projects::Directory).text().not_null())
                    .col(
                        ColumnDef::new(Projects::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(Projects::Table, Projects::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        add_sync_columns(manager, "projects").await?;

        manager
            .create_table(
                Table::create()
                    .table(Threads::Table)
                    .col(ColumnDef::new(Threads::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(Threads::UserId).uuid().not_null())
                    .col(ColumnDef::new(Threads::ProjectId).uuid().not_null())
                    .col(ColumnDef::new(Threads::Title).text().not_null())
                    .col(ColumnDef::new(Threads::Agent).text().not_null())
                    .col(ColumnDef::new(Threads::ConversationId).uuid().not_null())
                    .col(
                        ColumnDef::new(Threads::ConversationStarted)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Threads::Status)
                            .text()
                            .not_null()
                            .default("idle"),
                    )
                    .col(
                        ColumnDef::new(Threads::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(Threads::Table, Threads::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(Threads::Table, Threads::ProjectId)
                            .to(Projects::Table, Projects::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        add_sync_columns(manager, "threads").await?;

        manager
            .create_table(
                Table::create()
                    .table(ThreadEntries::Table)
                    .col(
                        ColumnDef::new(ThreadEntries::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ThreadEntries::UserId).uuid().not_null())
                    .col(ColumnDef::new(ThreadEntries::ThreadId).uuid().not_null())
                    .col(
                        ColumnDef::new(ThreadEntries::Position)
                            .big_integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(ThreadEntries::Kind).text().not_null())
                    .col(ColumnDef::new(ThreadEntries::Body).json_binary().not_null())
                    .col(
                        ColumnDef::new(ThreadEntries::CreatedAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(ThreadEntries::Table, ThreadEntries::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(ThreadEntries::Table, ThreadEntries::ThreadId)
                            .to(Threads::Table, Threads::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_thread_entries_thread_position")
                    .table(ThreadEntries::Table)
                    .col(ThreadEntries::ThreadId)
                    .col(ThreadEntries::Position)
                    .unique()
                    .to_owned(),
            )
            .await?;
        add_sync_columns(manager, "thread_entries").await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for table in ["thread_entries", "threads", "projects"] {
            remove_sync_columns(manager, table).await?;
        }
        manager
            .drop_table(Table::drop().table(ThreadEntries::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Threads::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Projects::Table).to_owned())
            .await
    }
}
