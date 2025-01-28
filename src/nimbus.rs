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
    secret::{EndpointSettings, PortBinding, RestartPolicyNameEnum},
    volume::CreateVolumeOptions,
    Docker,
};
use futures::StreamExt;
use std::collections::HashMap;

const NIMBUS_IMAGE: &str = "statusim/nimbus-eth2:amd64-latest";
const DEFAULT_P2P_TCP_PORT: u16 = 9000;
const DEFAULT_P2P_UDP_PORT: u16 = 9000;
const DEFAULT_REST_PORT: u16 = 5052;
const DEFAULT_METRICS_PORT: u16 = 8008;

#[derive(Debug, Clone)]
pub struct NimbusConfig {
    pub p2p_tcp_port: u16,
    pub p2p_udp_port: u16,
    pub rest_port: u16,
    pub metrics_port: u16,
    pub data_dir: String,
    pub execution_endpoint: String,
    pub network: String,
}

impl Default for NimbusConfig {
    fn default() -> Self {
        Self {
            p2p_tcp_port: DEFAULT_P2P_TCP_PORT,
            p2p_udp_port: DEFAULT_P2P_UDP_PORT,
            rest_port: DEFAULT_REST_PORT,
            metrics_port: DEFAULT_METRICS_PORT,
            data_dir: "/data".to_string(),
            execution_endpoint: "http://reth:8551".to_string(),
            network: "mainnet".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct NimbusNode {
    docker: Arc<Docker>,
    container_id: Arc<Mutex<Option<String>>>,
    config: NimbusConfig,
}

impl NimbusNode {
    pub async fn new(config: NimbusConfig) -> crate::Result<Self> {
        logging::info!("Initializing Nimbus node");
        let docker = Docker::connect_with_local_defaults().map_err(Error::Docker)?;
        let docker = Arc::new(docker);

        // Pull image if not present
        if let Err(_) = docker.inspect_image(NIMBUS_IMAGE).await {
            logging::info!("Pulling Nimbus image...");
            let mut pull_stream = docker.create_image(
                Some(CreateImageOptions {
                    from_image: NIMBUS_IMAGE,
                    ..Default::default()
                }),
                None,
                None,
            );

            while let Some(result) = pull_stream.next().await {
                match result {
                    Ok(output) => logging::debug!("Pull status: {:?}", output),
                    Err(e) => return Err(Error::Docker(e)),
                }
            }
        }

        let node = Self {
            docker,
            container_id: Arc::new(Mutex::new(None)),
            config,
        };

        Ok(node)
    }

    pub async fn create_container(&self) -> crate::Result<String> {
        // Create only Nimbus data volume with correct permissions
        if let Err(_) = self.docker.inspect_volume("nimbus_data").await {
            self.docker
                .create_volume(CreateVolumeOptions {
                    name: "nimbus_data".to_string(),
                    driver_opts: HashMap::from([
                        ("type".to_string(), "none".to_string()),
                        ("device".to_string(), "/data".to_string()),
                        ("o".to_string(), "bind".to_string()),
                    ]),
                    ..Default::default()
                })
                .await
                .map_err(Error::Docker)?;
        }

        let config = Config {
            image: Some(NIMBUS_IMAGE.to_string()),
            user: Some("root".to_string()),
            cmd: Some(vec![
                format!("--network={}", self.config.network),
                format!("--data-dir={}", self.config.data_dir),
                format!("--el={}", self.config.execution_endpoint),
                "--jwt-secret=/jwt/jwt.hex".into(),
                format!("--tcp-port={}", self.config.p2p_tcp_port),
                format!("--udp-port={}", self.config.p2p_udp_port),
                "--rest".into(),
                "--rest-address=0.0.0.0".into(),
                format!("--rest-port={}", self.config.rest_port),
                "--metrics".into(),
                "--metrics-address=0.0.0.0".into(),
                format!("--metrics-port={}", self.config.metrics_port),
                "--enr-auto-update=true".into(),
                "--log-level=info".into(),
                "--non-interactive=true".into(),
            ]),
            host_config: Some(HostConfig {
                binds: Some(vec!["nimbus_data:/data".into(), "reth_jwt:/jwt:ro".into()]),
                network_mode: Some("eth_network".to_string()),
                privileged: Some(true),
                port_bindings: Some(HashMap::from([
                    (
                        format!("{}/tcp", self.config.p2p_tcp_port),
                        Some(vec![PortBinding {
                            host_ip: Some("0.0.0.0".into()),
                            host_port: Some(self.config.p2p_tcp_port.to_string()),
                        }]),
                    ),
                    (
                        format!("{}/udp", self.config.p2p_udp_port),
                        Some(vec![PortBinding {
                            host_ip: Some("0.0.0.0".into()),
                            host_port: Some(self.config.p2p_udp_port.to_string()),
                        }]),
                    ),
                    (
                        format!("{}/tcp", self.config.rest_port),
                        Some(vec![PortBinding {
                            host_ip: Some("127.0.0.1".into()),
                            host_port: Some(self.config.rest_port.to_string()),
                        }]),
                    ),
                    (
                        format!("{}/tcp", self.config.metrics_port),
                        Some(vec![PortBinding {
                            host_ip: Some("127.0.0.1".into()),
                            host_port: Some(self.config.metrics_port.to_string()),
                        }]),
                    ),
                ])),
                restart_policy: Some(bollard::models::RestartPolicy {
                    name: Some(RestartPolicyNameEnum::UNLESS_STOPPED),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            networking_config: Some(NetworkingConfig {
                endpoints_config: HashMap::from([(
                    "eth_network".to_string(),
                    EndpointSettings {
                        aliases: Some(vec!["nimbus".into()]),
                        ..Default::default()
                    },
                )]),
            }),
            ..Default::default()
        };

        let container = self
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: "nimbus-eth2",
                    platform: Some("linux/amd64"),
                    ..Default::default()
                }),
                config,
            )
            .await
            .map_err(Error::Docker)?;

        Ok(container.id)
    }

    pub async fn initialize(&mut self) -> crate::Result<()> {
        logging::info!("Initializing Nimbus container");
        let mut container_id = self.container_id.lock().await;
        if container_id.is_none() {
            *container_id = Some(self.create_container().await?);
            logging::info!("Created Nimbus container");
        }
        Ok(())
    }

    pub async fn start_container(&self) -> crate::Result<()> {
        logging::info!("Starting Nimbus container");
        let id = self.container_id.lock().await;
        if let Some(id) = id.as_ref() {
            self.docker
                .start_container(id, None::<StartContainerOptions<String>>)
                .await
                .map_err(Error::Docker)?;
            logging::info!("Nimbus container started");
        }
        Ok(())
    }

    fn parse_container_log(log: bollard::container::LogOutput) -> String {
        match log {
            bollard::container::LogOutput::StdOut { message }
            | bollard::container::LogOutput::StdErr { message } => {
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

            match &info.state {
                Some(state) => {
                    logging::info!("Container state: {:?}", state);

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

                    if let Some(code) = state.exit_code {
                        if code != 0 {
                            logging::error!("Container exited with code: {}", code);
                            return Ok(false);
                        }
                    }

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
                        logging::info!("NIMBUS: {}", formatted_log);
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

            Ok(true)
        } else {
            logging::error!("No container ID available");
            Ok(false)
        }
    }

    pub async fn wait_for_healthy(&self) -> crate::Result<()> {
        logging::info!("Waiting for Nimbus node to be healthy");
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
        logging::info!("Starting Nimbus node health monitoring");
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            if !self.check_health().await? {
                logging::error!("Nimbus node became unhealthy");
                return Err(Error::Container("Node became unhealthy".into()));
            }
            logging::debug!("Nimbus node health check passed");
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
        logging::info!("Stopping Nimbus container");
        if let Some(id) = self.container_id.lock().await.as_ref() {
            self.docker
                .stop_container(id, None)
                .await
                .map_err(Error::Docker)?;
            logging::info!("Nimbus container stopped");
        }
        Ok(())
    }

    pub async fn remove(&self) -> crate::Result<()> {
        logging::info!("Removing Nimbus container");
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
            logging::info!("Nimbus container removed");
        }
        Ok(())
    }

    pub async fn cleanup(&self) -> crate::Result<()> {
        logging::info!("Cleaning up Nimbus resources");

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
        for volume in ["nimbus_data"] {
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
impl BackgroundService for NimbusNode {
    async fn start(&self) -> Result<oneshot::Receiver<Result<(), RunnerError>>, RunnerError> {
        logging::info!("Starting Nimbus node background service");
        let (tx, rx) = oneshot::channel();
        let mut node = self.clone();

        tokio::spawn(async move {
            let result = async {
                logging::info!("Initializing Nimbus node");
                node.initialize().await?;

                logging::info!("Starting Nimbus container");
                node.start_container().await?;

                logging::info!("Waiting for Nimbus node to become healthy");
                node.wait_for_healthy().await?;

                logging::info!("Starting Nimbus node health monitoring");
                node.monitor_health().await
            }
            .await;

            logging::info!("Nimbus node background service completed");
            let _ = tx.send(result.map_err(|e| {
                logging::error!("Nimbus node background service error: {}", e);
                RunnerError::Other(e.to_string())
            }));
        });

        Ok(rx)
    }
}
