use erno::boot::boot;
use teryon::{boot_config, Migrator};

#[tokio::main]
async fn main() {
    boot::<Migrator, ()>(boot_config()).await;
}
