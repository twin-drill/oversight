use clap::Subcommand;
use oversight::config::Config;
use oversight::healing_loop::daemon;
use oversight::healing_loop::policy::Regime;
use oversight::healing_loop::runner;

#[derive(Subcommand)]
pub enum LoopCommands {
    /// Run a single pass of the healing loop
    Run {
        /// Show proposed changes without writing to KB
        #[arg(long)]
        dry_run: bool,

        /// Topic creation regime: aggressive, balanced, or conservative
        #[arg(long)]
        regime: Option<String>,
    },

    /// Start the healing loop as a foreground daemon
    Start {
        /// Override the polling interval in seconds
        #[arg(long)]
        interval: Option<u64>,
    },

    /// Show the current loop processing state
    Status,
}

/// Execute a loop subcommand. Returns exit code.
pub fn run_loop_command(
    command: &LoopCommands,
    config: &Config,
) -> i32 {
    match command {
        LoopCommands::Run { dry_run, regime } => cmd_run(config, *dry_run, regime.as_deref()),
        LoopCommands::Start { interval } => cmd_start(config, *interval),
        LoopCommands::Status => cmd_status(config),
    }
}

fn cmd_run(config: &Config, dry_run: bool, regime_str: Option<&str>) -> i32 {
    // Parse CLI regime override
    let cli_regime = match regime_str {
        Some(s) => match s.parse::<Regime>() {
            Ok(r) => Some(r),
            Err(e) => {
                eprintln!("Error: {e}");
                return 1;
            }
        },
        None => None,
    };

    let mut config = config.clone();

    // Apply CLI regime override to config
    if let Some(regime) = cli_regime {
        config.loop_config.regime = regime;
    }

    // Build and validate policy early, before starting the runtime
    let policy = match config.loop_config.build_dedupe_policy(None) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: Invalid dedupe configuration: {e}");
            return 1;
        }
    };

    if dry_run {
        eprintln!("Regime: {}", policy.policy_summary());
    }

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to create async runtime: {e}");
            return 1;
        }
    };

    match rt.block_on(runner::run_once(config, dry_run)) {
        Ok(result) => {
            println!("{}", result.summary());
            if !result.errors.is_empty() {
                1
            } else {
                0
            }
        }
        Err(e) => {
            eprintln!("Loop run failed: {e}");
            1
        }
    }
}

fn cmd_start(config: &Config, interval_override: Option<u64>) -> i32 {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to create async runtime: {e}");
            return 1;
        }
    };

    let mut config = config.clone();
    if let Some(interval) = interval_override {
        config.loop_config.interval_secs = interval;
    }

    match rt.block_on(daemon::run_daemon(config)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Daemon failed: {e}");
            1
        }
    }
}

fn cmd_status(config: &Config) -> i32 {
    // Show configured regime
    let policy_summary = match config.loop_config.build_dedupe_policy(None) {
        Ok(p) => p.policy_summary(),
        Err(_) => format!("{} (invalid overrides)", config.loop_config.regime),
    };
    println!("Regime: {}", policy_summary);

    match runner::show_status(None) {
        Ok(summary) => {
            println!("{summary}");
            0
        }
        Err(e) => {
            eprintln!("Failed to read loop state: {e}");
            1
        }
    }
}
