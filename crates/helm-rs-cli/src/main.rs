use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "helm-rs")]
#[command(about = "Import Helm charts into loadable Rust render modules")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Import {
        chart_dir: PathBuf,
        #[arg(long)]
        crate_dir: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Import {
            chart_dir,
            crate_dir,
        } => helm_rs_cli::import_chart(chart_dir, crate_dir),
    };

    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
