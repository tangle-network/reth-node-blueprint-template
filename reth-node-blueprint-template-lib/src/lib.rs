use blueprint_sdk::extract::Context;
use blueprint_sdk::tangle::extract::{Optional, TangleArg, TangleResult};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::string::String;
use tracing::{debug, error, info, instrument, trace, warn};

// Create modules
pub mod monitoring;

// The job IDs - only for state-changing operations
pub const RETH_START_JOB_ID: u32 = 1;
pub const RETH_STOP_JOB_ID: u32 = 2;

// Configuration for the Reth node
#[derive(Clone)]
pub struct RethConfig {
    pub submodule_path: PathBuf,
    pub block_tip: Option<String>,
    pub monitoring_port: u16,
    pub grafana_port: u16,
}

impl Default for RethConfig {
    fn default() -> Self {
        Self {
            submodule_path: PathBuf::from("local_reth"),
            block_tip: None,
            monitoring_port: 9000,
            grafana_port: 3000,
        }
    }
}

// Context struct for Reth operations
#[derive(Clone)]
pub struct RethContext {
    pub config: RethConfig,
}

impl RethContext {
    pub fn new(config: RethConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self::new(RethConfig::default())
    }
}

// Helper function to run a command in the submodule directory
pub fn run_command(context: &RethContext, cmd: &str, args: &[&str]) -> std::io::Result<String> {
    debug!(command = cmd, arguments = ?args, "Running command");

    let output = Command::new(cmd)
        .current_dir(&context.config.submodule_path)
        .args(args)
        .output()?;

    if output.status.success() {
        match String::from_utf8(output.stdout) {
            Ok(s) => {
                trace!(output = %s, "Command executed successfully");
                Ok(s)
            }
            Err(e) => {
                error!(error = %e, "Invalid UTF-8 in command output");
                Err(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            }
        }
    } else {
        let error_message = String::from_utf8(output.stderr)
            .unwrap_or_else(|_| "Invalid UTF-8 in stderr".to_string());

        error!(
            status = %output.status,
            error = %error_message,
            "Command failed"
        );

        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "Command failed with status {}: {}",
                output.status, error_message
            ),
        ))
    }
}

// Run a command and stream its output in real-time
pub fn run_command_with_logs(
    context: &RethContext,
    cmd: &str,
    args: &[&str],
) -> std::io::Result<()> {
    info!(command = cmd, arguments = ?args, "Running command with live logs");

    let mut child = Command::new(cmd)
        .current_dir(&context.config.submodule_path)
        .args(args)
        .stdout(Stdio::inherit()) // Direct stdout to parent process
        .stderr(Stdio::inherit()) // Direct stderr to parent process
        .spawn()?;

    // Wait for the command to finish
    let status = child.wait()?;

    if status.success() {
        info!("Command completed successfully");
        Ok(())
    } else {
        error!(status = %status, "Command failed");
        Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Command failed with status: {}", status),
        ))
    }
}

// Start the Reth node - This is a state-changing operation (JOB)
#[instrument(skip(ctx), fields(block_tip = ?block_tip))]
pub async fn reth_start(
    Context(ctx): Context<RethContext>,
    TangleArg(Optional(block_tip)): TangleArg<Optional<String>>,
) -> TangleResult<String> {
    info!("Starting Reth node");

    // Set the block tip environment variable if provided
    if let Some(block_tip) = block_tip.as_ref().or(ctx.config.block_tip.as_ref()) {
        debug!(block_tip = %block_tip, "Setting custom block tip");

        // Use unsafe block for the environment variable setting
        unsafe {
            std::env::set_var("RETH_TIP", block_tip);
        }
    }

    info!("Running docker-compose up");

    // First check if the containers are already running
    let status_result = run_command(&ctx, "docker-compose", &["ps", "-q"]);
    match status_result {
        Ok(output) if !output.trim().is_empty() => {
            info!("Containers already running, showing logs");
            // Just show logs if already running
            match run_command_with_logs(&ctx, "docker-compose", &["logs", "--follow"]) {
                Ok(_) => {}
                Err(e) => warn!(error = %e, "Failed to follow logs of running containers"),
            }
        }
        _ => {
            // Start containers with direct log output
            println!("\n--- Starting Reth node with Docker Compose ---");
            if let Err(e) = run_command_with_logs(&ctx, "docker-compose", &["up"]) {
                error!(error = %e, "Failed to start Reth node");
                return TangleResult(format!("Failed to start Reth node: {}", e));
            }
        }
    }

    // Include the public URLs in the response
    let grafana_url = format!("http://localhost:{}", ctx.config.grafana_port);
    let prometheus_url = "http://localhost:9090";
    let metrics_url = format!("http://localhost:{}", ctx.config.monitoring_port);

    info!(
        grafana_url = %grafana_url,
        prometheus_url = %prometheus_url,
        metrics_url = %metrics_url,
        "Monitoring URLs"
    );

    TangleResult(format!(
        "Reth node started successfully.\n\nMonitoring dashboard available at: {}\nLogin with username: admin, password: admin\nPrometheus: {}\nMetrics endpoint: {}",
        grafana_url, prometheus_url, metrics_url
    ))
}

// Stop the Reth node - This is a state-changing operation (JOB)
#[instrument(skip(ctx))]
pub async fn reth_stop(Context(ctx): Context<RethContext>) -> TangleResult<String> {
    info!("Stopping Reth node");

    println!("\n--- Stopping Reth node with Docker Compose ---");

    // Run docker-compose down with direct log output
    match run_command_with_logs(&ctx, "docker-compose", &["down", "--volumes"]) {
        Ok(_) => {
            info!("Reth node stopped successfully");
            TangleResult(
                "Reth node stopped successfully. All containers and volumes removed.".to_string(),
            )
        }
        Err(e) => {
            error!(error = %e, "Failed to stop Reth node");
            TangleResult(format!("Failed to stop Reth node: {}", e))
        }
    }
}
