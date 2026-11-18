use erno::database::migrations::erno_migrations;
use sea_orm_migration::{MigrationTrait, MigratorTrait};

/// erno's own tables. Teryon's server adds none of its own yet.
pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        erno_migrations()
    }
}
