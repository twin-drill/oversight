use crate::cli::integrate::{IntegrateCommands, run_integrate_command};
use crate::cli::loop_cmd::{LoopCommands, run_loop_command};
use oversight::config::Config;
use oversight::integrate::{manager as integrate_manager, render as integrate_render, targets as integrate_targets};
use oversight::kb::frontmatter;
use oversight::KBService;
use oversight::TopicSummary;
use std::io::{self, IsTerminal, Read};
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "oversight", version, about = "Agent knowledge base and healing loop")]
pub struct Cli {
    /// Override the KB root directory
    #[arg(long, global = true)]
    pub kb_path: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize the knowledge base directory structure
    Init,

    /// List all topics in the knowledge base
    Topics {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Read a topic by slug or alias
    Read {
        /// Topic slug or alias
        topic: String,

        /// Include YAML frontmatter in output
        #[arg(long)]
        raw: bool,
    },

    /// Add a new topic (reads body from stdin)
    Add {
        /// Topic name (will be slugified)
        name: String,

        /// Tags for the topic (can be specified multiple times)
        #[arg(long = "tag", short = 't')]
        tags: Vec<String>,

        /// Aliases for the topic (can be specified multiple times)
        #[arg(long = "alias", short = 'a')]
        aliases: Vec<String>,
    },

    /// Update an existing topic's body (reads from stdin)
    Update {
        /// Topic slug or alias
        name: String,
    },

    /// Search topics by keyword
    Search {
        /// Search query
        query: String,
    },

    /// Delete a topic
    Delete {
        /// Topic slug or alias
        name: String,
    },

    /// Inject the oversight managed block into the current directory's agent config files
    Inject,

    /// Healing loop commands (discover, extract, merge)
    Loop {
        #[command(subcommand)]
        command: LoopCommands,
    },

    /// Agent integration commands (install, refresh, remove, status)
    Integrate {
        #[command(subcommand)]
        command: IntegrateCommands,
    },
}

/// Execute the CLI command. Returns exit code.
pub fn run(cli: Cli) -> i32 {
    // Loop and Integrate commands need the full config, not just KBService
    if let Commands::Loop { ref command } = cli.command {
        let config = match Config::resolve(cli.kb_path.as_deref()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error: {e}");
                return 1;
            }
        };
        return run_loop_command(command, &config);
    }

    if let Commands::Integrate { ref command } = cli.command {
        let config = match Config::resolve(cli.kb_path.as_deref()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error: {e}");
                return 1;
            }
        };
        return run_integrate_command(command, &config);
    }

    let service = match KBService::from_defaults(cli.kb_path.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };

    match cli.command {
        Commands::Init => cmd_init(&service),
        Commands::Topics { json } => cmd_topics(&service, json),
        Commands::Read { topic, raw } => cmd_read(&service, &topic, raw),
        Commands::Add {
            name,
            tags,
            aliases,
        } => cmd_add(&service, &name, tags, aliases),
        Commands::Update { name } => cmd_update(&service, &name),
        Commands::Search { query } => cmd_search(&service, &query),
        Commands::Delete { name } => cmd_delete(&service, &name),
        Commands::Inject => cmd_inject(),
        Commands::Loop { .. } => unreachable!(),
        Commands::Integrate { .. } => unreachable!(),
    }
}

fn cmd_init(service: &KBService) -> i32 {
    match service.init() {
        Ok(()) => {
            println!("Knowledge base initialized at {}", service.config().kb_path.display());
            0
        }
        Err(e) => {
            eprintln!("Error initializing KB: {e}");
            1
        }
    }
}

fn cmd_topics(service: &KBService, json: bool) -> i32 {
    match service.list_topics() {
        Ok(topics) => {
            if json {
                match serde_json::to_string_pretty::<Vec<TopicSummary>>(&topics) {
                    Ok(output) => {
                        println!("{output}");
                        0
                    }
                    Err(e) => {
                        eprintln!("Error serializing topics: {e}");
                        1
                    }
                }
            } else {
                if topics.is_empty() {
                    println!("No topics found. Use `oversight add` to create one.");
                    return 0;
                }
                for topic in &topics {
                    let aliases = if topic.aliases.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", topic.aliases.join(", "))
                    };
                    let tags = if topic.tags.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " {}",
                            topic.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ")
                        )
                    };
                    println!("{}{}{}", topic.slug, aliases, tags);
                }
                0
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

fn cmd_read(service: &KBService, topic: &str, raw: bool) -> i32 {
    match service.get_topic(topic) {
        Ok(t) => {
            if raw {
                match frontmatter::serialize(&t) {
                    Ok(content) => print!("{content}"),
                    Err(e) => {
                        eprintln!("Error serializing topic: {e}");
                        return 1;
                    }
                }
            } else {
                print!("{}", t.body);
            }
            0
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

fn cmd_add(service: &KBService, name: &str, tags: Vec<String>, aliases: Vec<String>) -> i32 {
    let body = match read_stdin() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error reading stdin: {e}");
            return 1;
        }
    };

    match service.add_topic(name, &body, tags, aliases) {
        Ok(topic) => {
            println!("Added topic: {} ({})", topic.title, topic.slug);
            0
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

fn cmd_update(service: &KBService, name: &str) -> i32 {
    let body = match read_stdin() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error reading stdin: {e}");
            return 1;
        }
    };

    match service.update_topic(name, &body) {
        Ok(topic) => {
            println!("Updated topic: {} ({})", topic.title, topic.slug);
            0
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

fn cmd_search(service: &KBService, query: &str) -> i32 {
    match service.search_topics(query) {
        Ok(results) => {
            if results.is_empty() {
                println!("No topics found matching \"{query}\".");
                return 0;
            }
            for result in &results {
                let topic = &result.topic;
                let tags = if topic.tags.is_empty() {
                    String::new()
                } else {
                    format!(
                        " {}",
                        topic.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ")
                    )
                };
                println!("{} - {}{}", topic.slug, topic.title, tags);
            }
            0
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

fn cmd_delete(service: &KBService, name: &str) -> i32 {
    match service.delete_topic(name) {
        Ok(()) => {
            println!("Deleted topic: {name}");
            0
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

/// Read all of stdin into a string.
fn read_stdin() -> io::Result<String> {
    let mut buf = String::new();
    if std::io::stdin().is_terminal() {
        eprintln!("Enter topic content (Ctrl+D to finish):");
    }
    io::stdin().read_to_string(&mut buf)?;
    Ok(buf)
}

fn cmd_inject() -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error: could not determine current directory: {e}");
            return 1;
        }
    };

    let files = ["CLAUDE.md", "AGENTS.md"];
    let target = integrate_targets::IntegrationTarget::claude_code();
    let block = integrate_render::render_managed_block(&target);
    let mut injected = 0;

    for filename in &files {
        let path = cwd.join(filename);
        match integrate_manager::install_block_at(&path, &target.identifier, &block) {
            Ok(action) => {
                println!("{}: {action}", path.display());
                injected += 1;
            }
            Err(e) => {
                eprintln!("Error writing {}: {e}", path.display());
            }
        }
    }

    if injected == 0 { 1 } else { 0 }
}
