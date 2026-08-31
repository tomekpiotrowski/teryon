//! Shared request-spec helpers. See the Erno Testing guide.

pub use erno::tests::{bearer, no_fixtures, setup_test, unverified_user, verified_user, TestUtils};

use teryon::boot_config;
use teryon::Migrator;

pub async fn setup() -> TestUtils {
    setup_test::<Migrator, _>(boot_config(), no_fixtures).await
}
