mod m20261114_000001_create_workspace;

use erno::database::migrations::erno_migrations;
use sea_orm_migration::{MigrationTrait, MigratorTrait};

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        erno_migrations()
            .into_iter()
            .chain([
                Box::new(m20261114_000001_create_workspace::Migration) as Box<dyn MigrationTrait>
            ])
            .collect()
    }
}
