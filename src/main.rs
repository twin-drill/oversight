mod cli;

use clap::Parser;
use cli::kb::{Cli, run};
use std::process;

fn main() {
    let cli = Cli::parse();
    let exit_code = run(cli);
    process::exit(exit_code);
}
