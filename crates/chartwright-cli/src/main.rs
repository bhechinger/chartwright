use std::io::Write;
use std::path::PathBuf;

use chartwright_abi::RenderRequest;
use clap::{Parser, Subcommand};
use serde_json::json;

#[derive(Debug, Parser)]
#[command(name = "chartwright")]
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
    Run {
        library_path: PathBuf,
        #[arg(long, default_value = "demo")]
        release_name: String,
        #[arg(long, default_value = "default")]
        namespace: String,
        #[arg(long)]
        values: Option<PathBuf>,
        /// Kubernetes version to target (e.g. "1.30.0"). Keep aligned with the
        /// oldest cluster version still receiving upstream support.
        #[arg(long, default_value = "1.30.0")]
        kube_version: String,
        #[arg(long = "api-version")]
        api_versions: Vec<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    let result: Result<(), Box<dyn std::error::Error>> = match cli.command {
        Command::Import {
            chart_dir,
            crate_dir,
        } => chartwright_cli::import_chart_with_events(
            chart_dir,
            crate_dir,
            chartwright_cli::StderrEventSink,
        )
        .map_err(Into::into),
        Command::Run {
            library_path,
            release_name,
            namespace,
            values,
            kube_version,
            api_versions,
        } => run_chart_module(
            library_path,
            release_name,
            namespace,
            values,
            kube_version,
            api_versions,
        )
        .map_err(Into::into),
    };

    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run_chart_module(
    library_path: PathBuf,
    release_name: String,
    namespace: String,
    values: Option<PathBuf>,
    kube_version: String,
    api_versions: Vec<String>,
) -> Result<(), chartwright_cli::RunError> {
    let values = match values {
        Some(path) => chartwright_cli::values_from_file(path),
        None => Ok(json!({})),
    }?;
    let rendered = chartwright_cli::run_chart_module(
        library_path,
        RenderRequest {
            release_name,
            namespace,
            values,
            kube_version,
            api_versions,
        },
    )?;
    std::io::stdout()
        .write_all(rendered.as_bytes())
        .map_err(|source| chartwright_cli::RunError::Io {
            path: "stdout".to_owned(),
            source,
        })
}
