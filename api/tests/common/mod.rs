//! Shared request-spec helpers. See the Erno Testing guide.
//!
//! Each test file compiles this module for itself and uses only part of it.
#![allow(dead_code)]

pub use erno::tests::{no_fixtures, setup_test, TestUtils};

use teryon_server::{boot_config, Migrator, Settings};

pub async fn setup() -> TestUtils {
    setup_test::<Migrator, Settings>(boot_config(), no_fixtures).await
}
