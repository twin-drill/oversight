use clap::Subcommand;
use oversight::config::Config;
use oversight::integrate::manager;
use oversight::integrate::targets;
use oversight::KBService;
use std::path::PathBuf;

/// Default target identifier when none is specified.
const DEFAULT_TARGET: &str = "claude-code";

#[derive(Subcommand)]
pub enum IntegrateCommands {
    /// Install managed block into agent config file
    Install {
        /// Target agent framework
        #[arg(long, default_value = DEFAULT_TARGET)]
        target: String,

        /// Override the target file path
        #[arg(long)]
        path: Option<PathBuf>,

        /// Show what would change without writing
        #[arg(long)]
        dry_run: bool,
    },

    /// Refresh topic list in installed targets
    Refresh {
        /// Target agent framework (refreshes all installed if omitted)
        #[arg(long)]
        target: Option<String>,

        /// Override the target file path
        #[arg(long)]
        path: Option<PathBuf>,

        /// Show what would change without writing
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove managed block from agent config file
    Remove {
        /// Target agent framework
        #[arg(long, default_value = DEFAULT_TARGET)]
        target: String,

        /// Override the target file path
        #[arg(long)]
        path: Option<PathBuf>,
    },

    /// Show integration status for targets
    Status,
}

/// Execute an integrate subcommand. Returns exit code.
pub fn run_integrate_command(
    command: &IntegrateCommands,
    config: &Config,
) -> i32 {
    match command {
        IntegrateCommands::Install {
            target,
            path,
            dry_run,
        } => cmd_install(config, target, path.as_deref(), *dry_run),
        IntegrateCommands::Refresh { target, path, dry_run } => {
            cmd_refresh(config, target.as_deref(), path.as_deref(), *dry_run)
        }
        IntegrateCommands::Remove { target, path } => {
            cmd_remove(target, path.as_deref())
        }
        IntegrateCommands::Status => cmd_status(),
    }
}

fn cmd_install(config: &Config, target_id: &str, path_override: Option<&std::path::Path>, dry_run: bool) -> i32 {
    let target = match targets::resolve_target(target_id) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };

    let topics = match load_topics_or_empty(config) {
        Ok(topics) => topics,
        Err(e) => {
            eprintln!("Error loading topics: {e}");
            return 1;
        }
    };
    let preview_limit = config.integrate.topic_preview_limit;

    match manager::install(&target, path_override, &topics, Some(preview_limit), dry_run) {
        Ok(result) => {
            println!("{}: {}", result.path.display(), result.action);
            0
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

fn cmd_refresh(config: &Config, target_id: Option<&str>, path_override: Option<&std::path::Path>, dry_run: bool) -> i32 {
    let topics = match load_topics_or_empty(config) {
        Ok(topics) => topics,
        Err(e) => {
            eprintln!("Error loading topics: {e}");
            return 1;
        }
    };
    let preview_limit = config.integrate.topic_preview_limit;

    let target_ids: Vec<&str> = match target_id {
        Some(id) => vec![id],
        None => vec!["claude-code"],
    };

    let mut exit_code = 0;
    for id in target_ids {
        let target = match targets::resolve_target(id) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Error resolving target '{id}': {e}");
                exit_code = 1;
                continue;
            }
        };

        match manager::refresh(&target, path_override, &topics, Some(preview_limit), dry_run) {
            Ok(result) => {
                println!("{}: {}", result.path.display(), result.action);
            }
            Err(e) => {
                eprintln!("Error refreshing target '{id}': {e}");
                exit_code = 1;
            }
        }
    }
    exit_code
}

fn cmd_remove(target_id: &str, path_override: Option<&std::path::Path>) -> i32 {
    let target = match targets::resolve_target(target_id) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };

    match manager::remove(&target, path_override, false) {
        Ok(result) => {
            println!("{}: {}", result.path.display(), result.action);
            0
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

fn cmd_status() -> i32 {
    let target_ids = vec!["claude-code"];
    for id in target_ids {
        match targets::resolve_target(id) {
            Ok(target) => {
                let st = manager::status(&target, None);
                print!("{st}");
            }
            Err(e) => {
                eprintln!("Error resolving target '{id}': {e}");
            }
        }
    }
    0
}

/// Load topics from KB, returning empty vec if KB is not initialized.
fn load_topics_or_empty(config: &Config) -> oversight::error::Result<Vec<oversight::TopicSummary>> {
    let service = KBService::new(config.clone());
    if !service.is_initialized() {
        return Ok(Vec::new());
    }
    service.list_topics()
}
