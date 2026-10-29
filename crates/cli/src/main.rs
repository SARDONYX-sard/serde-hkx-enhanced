pub mod args;
mod logger;
mod ui;

use crate::args::Args;
use clap::Parser;
use std::process::exit;
use tokio::time::Instant;

type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() {
    let start = Instant::now();

    match crate::args::run(Args::parse()).await {
        Ok(()) => {
            let elapsed = start.elapsed();
            let time = (elapsed.as_secs(), elapsed.subsec_millis());
            tracing::info!("Elapsed time: {}.{}secs.", time.0, time.1);
            exit(0);
        }
        Err(err) => {
            tracing::error!("{err}");
            eprintln!("{}", ui::error(&err.to_string()));
            exit(1);
        }
    }
}
