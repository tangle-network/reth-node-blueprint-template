use crate::Error;
use async_trait::async_trait;
use blueprint_sdk::{
    logging,
    runners::core::{error::RunnerError, runner::BackgroundService},
    std::sync::Arc,
    tokio::{
        self,
        sync::{oneshot, Mutex},
    },
};
use bollard::{
    container::{
        Config, CreateContainerOptions, InspectContainerOptions, LogsOptions, NetworkingConfig,
        RemoveContainerOptions, StartContainerOptions,
    },
    image::CreateImageOptions,
    models::HostConfig,
    secret::{EndpointSettings, PortBinding},
    Docker,
};
use futures::StreamExt;
use std::collections::HashMap;
use std::str;

// Constants for default configuration
const RETH_IMAGE: &str = "ghcr.io/paradigmxyz/reth:latest";
const DEFAULT_HTTP_PORT: u16 = 8545;
const DEFAULT_WS_PORT: u16 = 8546;
const DEFAULT_AUTH_PORT: u16 = 8551;
const DEFAULT_P2P_PORT: u16 = 30303;
const DEFAULT_METRICS_PORT: u16 = 9001;
const DEFAULT_BOOTNODES: [&str; 2] = [
    "enode://d860a01f9722d78051619d1e2351aba3f43f943f6f00718d1b9baa4101932a1f5011f16bb2b1bb35db20d6fe28fa0bf09636d26a87d31de9ec6203eeedb1f666@18.138.108.67:30303",
    "enode://22a8232c3abc76a16ae9d6c3b164f98775fe226f0917b0ca871128a74a8e9630b458460865bab457221f1d448dd9791d24c4e5d88786180ac185df813a68d4de@3.209.45.79:30303",
];

#[derive(Debug, Clone)]
pub struct RethConfig {
    pub http_port: u16,
    pub ws_port: u16,
    pub auth_port: u16,
    pub p2p_port: u16,
    pub metrics_port: u16,
    pub data_dir: String,
    pub jwt_secret_path: String,
    pub bootnodes: Vec<String>,
}

impl Default for RethConfig {
    fn default() -> Self {
        Self {
            http_port: DEFAULT_HTTP_PORT,
            ws_port: DEFAULT_WS_PORT,
            auth_port: DEFAULT_AUTH_PORT,
            p2p_port: DEFAULT_P2P_PORT,
            metrics_port: DEFAULT_METRICS_PORT,
            data_dir: "/data".to_string(),
            jwt_secret_path: "/jwt/jwt.hex".to_string(),
            bootnodes: DEFAULT_BOOTNODES.iter().map(|&s| s.to_string()).collect(),
        }
    }
}

#[derive(Clone)]
pub struct RethNode {
    docker: Arc<Docker>,
    container_id: Arc<Mutex<Option<String>>>,
    config: RethConfig,
}

impl RethNode {
    pub async fn new(config: RethConfig) -> crate::Result<Self> {
        logging::info!("Initializing RETH node");
        let docker = Docker::connect_with_local_defaults().map_err(Error::Docker)?;
        let docker = Arc::new(docker);

        // Pull image if not present
        if let Err(_) = docker.inspect_image(RETH_IMAGE).await {
            logging::info!("Pulling RETH image...");
            let mut pull_stream = docker.create_image(
                Some(CreateImageOptions {
                    from_image: RETH_IMAGE,
                    ..Default::default()
                }),
                None,
                None,
            );

            while let Some(result) = pull_stream.next().await {
                match result {
                    Ok(output) => {
                        if let Some(status) = output.status {
                            logging::debug!("Pull status: {}", status);
                        }
                    }
                    Err(e) => return Err(Error::Docker(e)),
                }
            }
            logging::info!("RETH image pulled successfully");
        }

        Ok(Self {
            docker,
            container_id: Arc::new(Mutex::new(None)),
            config,
        })
    }

    pub async fn initialize(&mut self) -> crate::Result<()> {
        logging::info!("Initializing RETH container");
        let mut container_id = self.container_id.lock().await;
        if container_id.is_none() {
            *container_id = Some(self.create_container().await?);
            logging::info!("Created RETH container");
        }
        Ok(())
    }

    pub async fn create_container(&self) -> crate::Result<String> {
        let config = Config {
            image: Some(RETH_IMAGE.to_string()),
            cmd: Some(vec![
                "node".into(),
                "--chain=mainnet".into(),
                format!("--datadir={}", self.config.data_dir),
                format!("--authrpc.jwtsecret={}", self.config.jwt_secret_path),
                "--authrpc.addr=0.0.0.0".into(),
                format!("--authrpc.port={}", self.config.auth_port),
                "--http".into(),
                "--http.api=debug,eth,net,trace,txpool,web3,rpc,reth,ots".into(),
                "--http.addr=0.0.0.0".into(),
                format!("--http.port={}", self.config.http_port),
                "--http.corsdomain=*".into(),
                "--ws".into(),
                "--ws.api=debug,eth,net,trace,txpool,web3,rpc,reth,ots".into(),
                "--ws.addr=0.0.0.0".into(),
                format!("--ws.port={}", self.config.ws_port),
                "--ws.origins=*".into(),
                format!("--port={}", self.config.p2p_port),
            ]),
            host_config: Some(HostConfig {
                binds: Some(vec!["reth_data:/data".into(), "reth_jwt:/jwt:ro".into()]),
                network_mode: Some("eth_network".into()),
                port_bindings: Some(HashMap::from([
                    (
                        format!("{}/tcp", self.config.http_port),
                        Some(vec![PortBinding {
                            host_ip: Some("0.0.0.0".into()),
                            host_port: Some(self.config.http_port.to_string()),
                        }]),
                    ),
                    (
                        format!("{}/tcp", self.config.p2p_port),
                        Some(vec![PortBinding {
                            host_ip: Some("0.0.0.0".into()),
                            host_port: Some(self.config.p2p_port.to_string()),
                        }]),
                    ),
                    (
                        format!("{}/udp", self.config.p2p_port),
                        Some(vec![PortBinding {
                            host_ip: Some("0.0.0.0".into()),
                            host_port: Some(self.config.p2p_port.to_string()),
                        }]),
                    ),
                ])),
                ..Default::default()
            }),
            networking_config: Some(NetworkingConfig {
                endpoints_config: HashMap::from([(
                    "eth_network".into(),
                    EndpointSettings {
                        aliases: Some(vec!["reth".into()]),
                        ..Default::default()
                    },
                )]),
            }),
            ..Default::default()
        };

        let container = self
            .docker
            .create_container(None::<CreateContainerOptions<String>>, config)
            .await
            .map_err(Error::Docker)?;

        Ok(container.id)
    }

    pub async fn start_container(&self) -> crate::Result<()> {
        logging::info!("Starting RETH container");
        let id = self.container_id.lock().await;
        if let Some(id) = id.as_ref() {
            self.docker
                .start_container(id, None::<StartContainerOptions<String>>)
                .await
                .map_err(Error::Docker)?;
            logging::info!("RETH container started");
        }
        Ok(())
    }

    fn parse_container_log(log: bollard::container::LogOutput) -> String {
        match log {
            bollard::container::LogOutput::StdOut { message }
            | bollard::container::LogOutput::StdErr { message } => {
                // Remove ANSI escape codes and convert to string
                String::from_utf8_lossy(&message)
                    .replace("\u{1b}[0m", "")
                    .replace("\u{1b}[32m", "")
                    .replace("\u{1b}[2m", "")
                    .trim()
                    .to_string()
            }
            _ => String::new(),
        }
    }

    pub async fn check_health(&self) -> crate::Result<bool> {
        if let Some(id) = self.container_id.lock().await.as_ref() {
            let info = self
                .docker
                .inspect_container(id, None::<InspectContainerOptions>)
                .await
                .map_err(Error::Docker)?;

            // Check container state
            match &info.state {
                Some(state) => {
                    logging::info!("Container state: {:?}", state);

                    // Check for OOM or other errors
                    if let Some(true) = state.oom_killed {
                        logging::error!("Container was OOM killed");
                        return Ok(false);
                    }

                    if let Some(error) = &state.error {
                        if !error.is_empty() {
                            logging::error!("Container error: {}", error);
                            return Ok(false);
                        }
                    }

                    // Check exit code if container has stopped
                    if let Some(code) = state.exit_code {
                        if code != 0 {
                            logging::error!("Container exited with code: {}", code);
                            return Ok(false);
                        }
                    }

                    // If not running, return false
                    if !state.running.unwrap_or(false) {
                        logging::warn!("Container is not running");
                        return Ok(false);
                    }
                }
                None => {
                    logging::error!("No container state information available");
                    return Ok(false);
                }
            }

            // Get logs with timestamps
            let mut logs = self.docker.logs(
                id,
                Some(LogsOptions::<String> {
                    stdout: true,
                    stderr: true,
                    follow: false,
                    timestamps: true,
                    tail: "50".to_string(),
                    ..Default::default()
                }),
            );

            let mut found_error = false;
            while let Some(log) = logs.next().await {
                match log {
                    Ok(log) => {
                        let formatted_log = Self::parse_container_log(log);
                        logging::info!("RETH : {}", formatted_log);
                        if formatted_log.contains("error") || formatted_log.contains("Error") {
                            found_error = true;
                            logging::error!("Found error in logs: {}", formatted_log);
                        }
                    }
                    Err(e) => {
                        logging::error!("Error reading log: {}", e);
                        found_error = true;
                    }
                }
            }

            if found_error {
                return Ok(false);
            }

            // If we got here and the container is running, consider it healthy
            Ok(true)
        } else {
            logging::error!("No container ID available");
            Ok(false)
        }
    }

    pub async fn wait_for_healthy(&self) -> crate::Result<()> {
        logging::info!("Waiting for RETH node to be healthy");
        let mut retries = 0;
        while retries < 30 {
            if self.check_health().await? {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            retries += 1;
        }
        Err(Error::Container("Node failed to become healthy".into()))
    }

    pub async fn monitor_health(self) -> crate::Result<()> {
        logging::info!("Starting RETH node health monitoring");
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            if !self.check_health().await? {
                logging::error!("RETH node became unhealthy");
                return Err(Error::Container("Node became unhealthy".into()));
            }
            logging::debug!("RETH node health check passed");
        }
    }

    pub async fn get_logs(
        &self,
    ) -> crate::Result<impl futures::Stream<Item = Result<String, Error>>> {
        if let Some(id) = self.container_id.lock().await.as_ref() {
            let logs = self
                .docker
                .logs(
                    id,
                    Some(LogsOptions::<String> {
                        stdout: true,
                        stderr: true,
                        follow: true,
                        ..Default::default()
                    }),
                )
                .map(|r| {
                    r.map_err(Error::Docker)
                        .and_then(|l| Ok(Self::parse_container_log(l)))
                });

            Ok(logs)
        } else {
            Err(Error::Container("Container not started".into()))
        }
    }

    pub async fn stop(&self) -> crate::Result<()> {
        logging::info!("Stopping RETH container");
        if let Some(id) = self.container_id.lock().await.as_ref() {
            self.docker
                .stop_container(id, None)
                .await
                .map_err(Error::Docker)?;
            logging::info!("RETH container stopped");
        }
        Ok(())
    }

    pub async fn remove(&self) -> crate::Result<()> {
        logging::info!("Removing RETH container");
        if let Some(id) = self.container_id.lock().await.as_ref() {
            self.docker
                .remove_container(
                    id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await
                .map_err(Error::Docker)?;
            logging::info!("RETH container removed");
        }
        Ok(())
    }

    pub async fn cleanup(&self) -> crate::Result<()> {
        logging::info!("Cleaning up RETH resources");

        // Stop and remove container if it exists
        if let Some(id) = self.container_id.lock().await.as_ref() {
            self.docker
                .remove_container(
                    id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await
                .map_err(Error::Docker)?;
        }

        // Remove volumes
        for volume in ["rethdata", "rethjwt"] {
            if let Ok(_) = self.docker.inspect_volume(volume).await {
                self.docker
                    .remove_volume(volume, None)
                    .await
                    .map_err(Error::Docker)?;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl BackgroundService for RethNode {
    async fn start(&self) -> Result<oneshot::Receiver<Result<(), RunnerError>>, RunnerError> {
        logging::info!("Starting RETH node background service");
        let (tx, rx) = oneshot::channel();
        let mut node = self.clone();

        tokio::spawn(async move {
            let result = async {
                logging::info!("Initializing RETH node");
                // Initialize if needed
                node.initialize().await?;

                logging::info!("Starting RETH container");
                // Start container
                node.start_container().await?;

                logging::info!("Waiting for RETH node to become healthy");
                // Wait for healthy
                node.wait_for_healthy().await?;

                logging::info!("Starting RETH node health monitoring");
                // Start background monitoring
                node.monitor_health().await
            }
            .await;

            logging::info!("RETH node background service completed");
            let _ = tx.send(result.map_err(|e| {
                logging::error!("RETH node background service error: {}", e);
                RunnerError::Other(e.to_string())
            }));
        });

        Ok(rx)
    }
}
