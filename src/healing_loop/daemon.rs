use crate::config::Config;
use crate::error::Result;
use crate::healing_loop::runner::Runner;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Run the healing loop as a foreground daemon with configurable interval.
///
/// Runs repeatedly until interrupted (Ctrl-C). Each iteration performs a
/// full discover -> reduce -> extract -> dedup -> merge cycle.
pub async fn run_daemon(config: Config) -> Result<()> {
    let interval_secs = config.loop_config.interval_secs;
    let runner = Runner::new(config);

    // Set up Ctrl-C handler
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\nReceived Ctrl-C, shutting down gracefully...");
            r.store(false, Ordering::SeqCst);
        }
    });

    eprintln!(
        "Healing loop daemon started (interval: {}s). Press Ctrl-C to stop.",
        interval_secs
    );

    let mut iteration = 0u64;
    while running.load(Ordering::SeqCst) {
        iteration += 1;
        eprintln!("\n--- Iteration {iteration} ---");

        match runner.run_once(false).await {
            Ok(result) => {
                eprintln!("{}", result.summary());
            }
            Err(e) => {
                eprintln!("Loop iteration failed: {e}");
                eprintln!("Will retry on next interval.");
            }
        }

        // Wait for the interval, but check running flag periodically
        let mut elapsed = 0u64;
        while elapsed < interval_secs && running.load(Ordering::SeqCst) {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            elapsed += 1;
        }
    }

    eprintln!("Daemon stopped.");
    Ok(())
}
