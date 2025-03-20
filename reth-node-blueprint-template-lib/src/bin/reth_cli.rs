use clap::{Parser, Subcommand};
use reth_docker_template_blueprint_lib::{RethConfig, RethContext, monitoring};
use std::path::PathBuf;
use std::process::ExitCode;
use tokio::runtime::Runtime;
use tracing::{debug, error, info, warn};

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Optional path to the local_reth directory
    #[arg(short, long)]
    path: Option<PathBuf>,

    /// Optional block tip for syncing
    #[arg(short, long)]
    block_tip: Option<String>,

    /// Grafana port
    #[arg(long, default_value_t = 3000)]
    grafana_port: u16,

    /// Monitoring port
    #[arg(long, default_value_t = 9000)]
    monitoring_port: u16,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Reth node
    Start,

    /// Stop the Reth node
    Stop,

    /// Get the status of the Reth node
    Status,

    /// Get logs from the Reth node
    Logs {
        /// Number of lines to display
        #[arg(short, long)]
        lines: Option<usize>,

        /// Follow the logs (stream logs to terminal)
        #[arg(short, long)]
        follow: bool,
    },

    /// Check if Grafana is ready
    Grafana,

    /// Get metrics from Prometheus
    Metrics,

    /// Get URLs for all services
    Urls,
}

// Setup logging
fn setup_logging(verbose: bool) {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::format::FmtSpan;

    let default_level = if verbose { "debug" } else { "info" };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(default_level.parse().unwrap())
                .from_env_lossy(),
        )
        .with_span_events(FmtSpan::NONE)
        .try_init();
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Setup logging
    setup_logging(cli.verbose);

    // Create context with user-provided configuration
    let mut config = RethConfig::default();
    if let Some(path) = cli.path {
        config.submodule_path = path;
    }
    if let Some(block_tip) = cli.block_tip.clone() {
        config.block_tip = Some(block_tip);
    }
    config.grafana_port = cli.grafana_port;
    config.monitoring_port = cli.monitoring_port;

    let context = RethContext::new(config);

    // Create runtime for async functions
    let rt = Runtime::new().expect("Failed to create Tokio runtime");

    match cli.command {
        Commands::Start => {
            println!("\n--- Starting Reth node ---");

            // Set block tip if provided
            if let Some(block_tip) = cli.block_tip {
                // Use unsafe block for environment variable setting
                unsafe {
                    std::env::set_var("RETH_TIP", block_tip);
                }
            }

            let result = rt.block_on(async {
                use blueprint_sdk::extract::Context;
                use blueprint_sdk::tangle::extract::TangleArg;
                use reth_docker_template_blueprint_lib::reth_start;

                reth_start(Context(context.clone()), TangleArg(None.into())).await
            });

            match result {
                result => {
                    println!("{}", result.0);

                    // Show service URLs
                    let urls = monitoring::get_service_urls(&context);

                    println!("Node started successfully. Run 'reth-cli logs -f' to follow logs.");
                }
            }
        }
        Commands::Stop => {
            println!("\n--- Stopping Reth node ---");

            let result = rt.block_on(async {
                use blueprint_sdk::extract::Context;
                use reth_docker_template_blueprint_lib::reth_stop;

                reth_stop(Context(context)).await
            });

            match result {
                result => println!("{}", result.0),
            }
        }
        Commands::Status => {
            let status = monitoring::get_status(&context);
            match status {
                Ok(output) => println!("{}", output),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    return ExitCode::FAILURE;
                }
            }
        }
        Commands::Logs { lines, follow } => {
            if follow {
                // This will be handled directly by run_command_with_logs in the lib.rs file
                let result = rt.block_on(async {
                    use blueprint_sdk::extract::Context;
                    use reth_docker_template_blueprint_lib::run_command_with_logs;

                    println!("\n--- Following Reth node logs (press Ctrl+C to stop) ---");
                    run_command_with_logs(&context, "docker-compose", &["logs", "--follow", "reth"])
                });

                if let Err(e) = result {
                    eprintln!("Failed to follow logs: {}", e);
                    return ExitCode::FAILURE;
                }
            } else {
                let logs = monitoring::get_logs(&context, lines);
                match logs {
                    Ok(output) => println!("{}", output),
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        return ExitCode::FAILURE;
                    }
                }
            }
        }
        Commands::Grafana => {
            let grafana = monitoring::check_grafana_ready(&context);
            match grafana {
                Ok(output) => println!("{}", output),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    return ExitCode::FAILURE;
                }
            }
        }
        Commands::Metrics => {
            let metrics = monitoring::get_metrics(&context);
            match metrics {
                Ok(metrics) => {
                    println!("\nMetrics from Reth node:");
                    for (key, value) in metrics.clone() {
                        if cli.verbose {
                            println!("  {}: {}", key, value);
                        }
                    }
                    println!("Retrieved {} metrics", metrics.len());
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    return ExitCode::FAILURE;
                }
            }
        }
        Commands::Urls => {
            let urls = monitoring::get_service_urls(&context);
            println!("Service URLs:");
            for (service, url) in urls {
                println!("  {}: {}", service, url);
            }
        }
    }

    ExitCode::SUCCESS
}
