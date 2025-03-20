use crate::{RethContext, run_command, run_command_with_logs};
use std::collections::HashMap;
use std::io;
use tracing::{debug, error, info, warn};

/// Get status of Reth node
pub fn get_status(context: &RethContext) -> Result<String, String> {
    println!("\n--- Checking Reth node status ---");

    // First try running with direct console output
    let _ = run_command_with_logs(context, "docker-compose", &["ps"]);

    // Then get the output as string for return value
    match run_command(context, "docker-compose", &["ps"]) {
        Ok(output) => {
            if output.trim().is_empty() {
                Ok("No Reth services are currently running.".to_string())
            } else {
                Ok(format!("Reth services status:\n{}", output))
            }
        }
        Err(e) => Err(format!("Failed to get Reth status: {}", e)),
    }
}

/// Get logs from the Reth node
pub fn get_logs(context: &RethContext, lines: Option<usize>) -> Result<String, String> {
    println!("\n--- Fetching Reth node logs ---");

    // Create command arguments with owned strings
    let mut cmd_args = vec!["logs".to_string()];

    if let Some(lines) = lines {
        cmd_args.push("--tail".to_string());
        cmd_args.push(lines.to_string());
    }

    cmd_args.push("reth".to_string());

    // Convert to string slice references for the command
    let args: Vec<&str> = cmd_args.iter().map(|s| s.as_str()).collect();

    // First show logs directly to console
    let _ = run_command_with_logs(context, "docker-compose", &args);

    // Then get output as string for return
    match run_command(context, "docker-compose", &args) {
        Ok(output) => {
            if output.trim().is_empty() {
                Ok("No logs available from Reth node.".to_string())
            } else {
                Ok(format!("Reth node logs:\n{}", output))
            }
        }
        Err(e) => Err(format!("Failed to get Reth logs: {}", e)),
    }
}

/// Check if Grafana is ready and return the URL
pub fn check_grafana_ready(context: &RethContext) -> Result<String, String> {
    println!("\n--- Checking Grafana status ---");

    // Display status directly to console
    let _ = run_command_with_logs(context, "docker-compose", &["ps", "grafana"]);

    // Check if Grafana container is running
    match run_command(context, "docker-compose", &["ps", "grafana"]) {
        Ok(status) => {
            if status.contains("Up") {
                Ok(format!(
                    "Grafana is running and available at http://localhost:{}\n\
                    Login with username: admin, password: admin\n\
                    The Reth dashboard should be available after login.",
                    context.config.grafana_port
                ))
            } else {
                Err("Grafana is not running. Please start the Reth node first.".to_string())
            }
        }
        Err(e) => Err(format!("Failed to check Grafana status: {}", e)),
    }
}

/// Get metrics from the Prometheus metrics endpoint
pub fn get_metrics(context: &RethContext) -> Result<HashMap<String, String>, String> {
    println!("\n--- Fetching metrics from Prometheus ---");

    // First check if the Reth node is running
    let running = match get_status(context) {
        Ok(status) => !status.contains("No Reth services"),
        Err(_) => false,
    };

    if !running {
        return Err("Reth node is not running. Please start it first.".to_string());
    }

    // Use curl to get metrics with direct output
    let endpoint = format!("localhost:{}", context.config.monitoring_port);

    // Show some metrics directly to console
    let _ = run_command_with_logs(context, "curl", &["-s", &endpoint]);

    // Parse metrics for return value
    match run_command(context, "curl", &["-s", &endpoint]) {
        Ok(output) => {
            // Parse the Prometheus metrics format
            let mut metrics = HashMap::new();
            for line in output.lines() {
                // Skip comments and empty lines
                if line.starts_with('#') || line.is_empty() {
                    continue;
                }

                // Try to parse "key value" format
                if let Some(pos) = line.find(' ') {
                    let key = &line[0..pos];
                    let value = &line[pos + 1..];
                    metrics.insert(key.to_string(), value.to_string());
                }
            }

            Ok(metrics)
        }
        Err(e) => Err(format!("Failed to get metrics: {}", e)),
    }
}

/// Get the URLs for accessing the services
pub fn get_service_urls(context: &RethContext) -> HashMap<String, String> {
    let mut urls = HashMap::new();

    urls.insert(
        "grafana".to_string(),
        format!("http://localhost:{}", context.config.grafana_port),
    );
    urls.insert(
        "prometheus".to_string(),
        "http://localhost:9090".to_string(),
    );
    urls.insert(
        "metrics".to_string(),
        format!("http://localhost:{}", context.config.monitoring_port),
    );

    println!("\n--- Service URLs ---");
    for (service, url) in &urls {
        println!("{}: {}", service, url);
    }

    urls
}
