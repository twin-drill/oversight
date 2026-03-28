use crate::config::Config;
use crate::error::Result;
use crate::healing_loop::runner::Runner;
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// Debounce window — coalesce rapid writes (agent sessions write many times).
const DEBOUNCE_SECS: u64 = 10;

/// Maximum time between forced polls even when using fs events,
/// as a safety net for missed events (NFS, FUSE, edge cases).
const MAX_POLL_INTERVAL_SECS: u64 = 900; // 15 minutes

/// Run the healing loop as a foreground daemon.
///
/// Prefers filesystem event notification (FSEvents/inotify via `notify` crate)
/// and falls back to interval polling if watching fails.
pub async fn run_daemon(config: Config) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("\nReceived Ctrl-C, shutting down gracefully...");
            r.store(false, Ordering::SeqCst);
        }
    });

    let source = config.build_source();
    let watch_paths = source.watch_paths();
    drop(source);

    match try_start_watcher(&watch_paths) {
        Some(rx) => {
            eprintln!(
                "Healing loop daemon started (fs-watch mode, {} path(s)). Press Ctrl-C to stop.",
                watch_paths.len()
            );
            for p in &watch_paths {
                eprintln!("  Watching: {}", p.display());
            }
            run_watch_loop(config, running, rx).await
        }
        None => {
            eprintln!(
                "Filesystem watcher unavailable, falling back to polling ({}s interval).",
                config.loop_config.interval_secs
            );
            run_poll_loop(config, running).await
        }
    }
}

/// Attempt to start the filesystem watcher. Returns a channel receiver if
/// successful, or None if watching fails (paths don't exist, unsupported FS, etc.).
fn try_start_watcher(
    watch_paths: &[PathBuf],
) -> Option<mpsc::UnboundedReceiver<()>> {
    let (tx, rx) = mpsc::unbounded_channel();

    let debounce_duration = Duration::from_secs(DEBOUNCE_SECS);

    let mut debouncer = new_debouncer(debounce_duration, move |res: std::result::Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
        match res {
            Ok(events) => {
                let dominated_by_rewrite = events.iter().all(|e| e.kind == DebouncedEventKind::Any);
                if dominated_by_rewrite && !events.is_empty() {
                    let _ = tx.send(());
                }
            }
            Err(e) => {
                eprintln!("Filesystem watcher error: {e}");
            }
        }
    })
    .ok()?;

    let mut any_watched = false;
    for path in watch_paths {
        if !path.exists() {
            eprintln!("  Watch path does not exist (skipping): {}", path.display());
            continue;
        }
        let mode = if path.is_dir() {
            notify::RecursiveMode::Recursive
        } else {
            notify::RecursiveMode::NonRecursive
        };
        match debouncer.watcher().watch(path, mode) {
            Ok(()) => {
                any_watched = true;
            }
            Err(e) => {
                eprintln!("  Failed to watch {}: {e}", path.display());
            }
        }
    }

    if !any_watched {
        return None;
    }

    // Leak the debouncer so it lives for the process lifetime.
    // The daemon runs until Ctrl-C, so this is intentional.
    std::mem::forget(debouncer);

    Some(rx)
}

/// Event-driven loop: runs pipeline on fs events, with a max-interval safety poll.
async fn run_watch_loop(
    config: Config,
    running: Arc<AtomicBool>,
    mut rx: mpsc::UnboundedReceiver<()>,
) -> Result<()> {
    let runner = Runner::new(config);
    let max_interval = Duration::from_secs(MAX_POLL_INTERVAL_SECS);
    let mut iteration = 0u64;

    // Run once at startup
    iteration += 1;
    eprintln!("\n--- Iteration {iteration} (startup) ---");
    run_iteration(&runner).await;

    while running.load(Ordering::SeqCst) {
        // Wait for either: fs event, max interval timeout, or shutdown
        let triggered = tokio::select! {
            Some(()) = rx.recv() => {
                // Drain any additional queued events (burst coalescing)
                while rx.try_recv().is_ok() {}
                "fs-event"
            }
            _ = tokio::time::sleep(max_interval) => {
                "scheduled"
            }
        };

        if !running.load(Ordering::SeqCst) {
            break;
        }

        iteration += 1;
        eprintln!("\n--- Iteration {iteration} ({triggered}) ---");
        run_iteration(&runner).await;
    }

    eprintln!("Daemon stopped.");
    Ok(())
}

/// Polling fallback loop — same behavior as the original daemon.
async fn run_poll_loop(config: Config, running: Arc<AtomicBool>) -> Result<()> {
    let interval_secs = config.loop_config.interval_secs;
    let runner = Runner::new(config);

    eprintln!(
        "Healing loop daemon started (poll mode, interval: {}s). Press Ctrl-C to stop.",
        interval_secs
    );

    let mut iteration = 0u64;
    while running.load(Ordering::SeqCst) {
        iteration += 1;
        eprintln!("\n--- Iteration {iteration} ---");
        run_iteration(&runner).await;

        let mut elapsed = 0u64;
        while elapsed < interval_secs && running.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_secs(1)).await;
            elapsed += 1;
        }
    }

    eprintln!("Daemon stopped.");
    Ok(())
}

async fn run_iteration(runner: &Runner) {
    match runner.run_once(false).await {
        Ok(result) => {
            eprintln!("{}", result.summary());
        }
        Err(e) => {
            eprintln!("Loop iteration failed: {e}");
            eprintln!("Will retry on next trigger.");
        }
    }
}
