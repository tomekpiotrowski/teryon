use erno::boot::boot;
use teryon::{boot_config, Migrator, Settings};

#[tokio::main]
async fn main() {
    boot::<Migrator, Settings>(boot_config()).await;
}
