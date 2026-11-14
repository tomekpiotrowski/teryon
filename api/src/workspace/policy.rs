//! Who may see what in the workspace: only the owner.
//!
//! One rule serves all three tables because each row carries its owner. That
//! is also what erno's synchronisation needs, since it decides who receives a
//! change from the changed row alone.

use erno::database::models::user;
use erno::policy::Policy;
use erno::sync::from_user::FromUser;
use sea_orm::{ColumnTrait, QueryFilter, Select};
use uuid::Uuid;

use crate::workspace::{project, thread, thread_entry};

pub struct OwnerPolicy {
    pub user_id: Uuid,
}

impl FromUser for OwnerPolicy {
    fn from_user(user: &user::Model) -> Self {
        Self { user_id: user.id }
    }
}

impl Policy<project::Entity> for OwnerPolicy {
    fn can_read(&self, project: &project::Model) -> bool {
        project.user_id == self.user_id
    }

    fn readable(&self, query: Select<project::Entity>) -> Select<project::Entity> {
        query.filter(project::Column::UserId.eq(self.user_id))
    }
}

impl Policy<thread::Entity> for OwnerPolicy {
    fn can_read(&self, thread: &thread::Model) -> bool {
        thread.user_id == self.user_id
    }

    fn readable(&self, query: Select<thread::Entity>) -> Select<thread::Entity> {
        query.filter(thread::Column::UserId.eq(self.user_id))
    }
}

impl Policy<thread_entry::Entity> for OwnerPolicy {
    fn can_read(&self, entry: &thread_entry::Model) -> bool {
        entry.user_id == self.user_id
    }

    fn readable(&self, query: Select<thread_entry::Entity>) -> Select<thread_entry::Entity> {
        query.filter(thread_entry::Column::UserId.eq(self.user_id))
    }
}
