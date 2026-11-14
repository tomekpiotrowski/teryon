//! A project: a directory on this machine that agents work in.

use chrono::NaiveDateTime;
use erno::sync::syncable::Syncable;
use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

use crate::workspace::policy::OwnerPolicy;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "projects")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub directory: String,
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
        "projects"
    }

    fn sync_seq_column() -> Column {
        Column::SyncSeq
    }

    fn sync_seq(model: &Model) -> i64 {
        model.sync_seq
    }
}
