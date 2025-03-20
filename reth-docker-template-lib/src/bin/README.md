# Reth CLI Tool

This is a standalone command-line tool for managing the Reth Ethereum node with Prometheus and Grafana monitoring. It provides direct interaction with the Reth node without going through the Tangle Blueprint system.

## Usage

```bash
reth-cli [OPTIONS] <COMMAND>
```

### Options

- `-p, --path <PATH>` - Optional path to the local_reth directory
- `-b, --block-tip <BLOCK_TIP>` - Optional block tip for syncing
- `--grafana-port <GRAFANA_PORT>` - Grafana port (default: 3000)
- `--monitoring-port <MONITORING_PORT>` - Monitoring port (default: 9000)
- `-v, --verbose` - Enable verbose logging (debug level)

### Commands

- `start` - Start the Reth node
- `stop` - Stop the Reth node
- `status` - Get the status of the Reth node
- `logs` - Get logs from the Reth node
  - `-l, --lines <LINES>` - Number of lines to display
  - `-f, --follow` - Follow the logs (stream in real-time)
- `grafana` - Check if Grafana is ready
- `metrics` - Get metrics from Prometheus
- `urls` - Get URLs for all services

## Examples

```bash
# Start the Reth node
reth-cli start

# Start with custom block tip
reth-cli -b "0x7d5a4369273c723454ac137f48a4f142b097aa2779464e6505f1b1c5e37b5382" start

# Get the status
reth-cli status

# View logs (last 100 lines)
reth-cli logs -l 100

# Stream logs in real-time with color coding by log level
reth-cli logs -f

# Follow logs with verbose output
reth-cli -v logs -f

# Check Grafana status
reth-cli grafana

# Get service URLs
reth-cli urls

# Stop the node
reth-cli stop
```

## Building

You can build the CLI tool with:

```bash
cargo build --bin reth-cli
```

The binary will be located at `target/debug/reth-cli` or `target/release/reth-cli` if built with the `--release` flag.

## Features

- **Color-coded logs**: Log output is color-coded by severity level (error, warn, info, debug)
- **Real-time log streaming**: Follow mode for logs shows real-time output
- **Formatted output**: Clean and structured output for all commands
- **Verbose mode**: Detailed logging for debugging

## Dependencies

- Docker and Docker Compose must be installed
- Access to the local_reth directory (automatically included as a submodule)
