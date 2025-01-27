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
        Config, CreateContainerOptions, InspectContainerOptions, LogsOptions,
        RemoveContainerOptions, StartContainerOptions,
    },
    exec::{CreateExecOptions, StartExecResults},
    image::CreateImageOptions,
    models::HostConfig,
    secret::PortBinding,
    volume::CreateVolumeOptions,
    Docker,
};
use futures::StreamExt;
use serde_json;
use std::collections::HashMap;
use std::io::Write;

const RETH_IMAGE: &str = "ghcr.io/paradigmxyz/reth:latest";
const DEFAULT_HTTP_PORT: u16 = 8543;
const DEFAULT_WS_PORT: u16 = 8544;
const DEFAULT_P2P_PORT: u16 = 30304;
const DEFAULT_AUTH_PORT: u16 = 8551;

#[derive(Debug, Clone)]
pub struct RethConfig {
    pub http_port: u16,
    pub ws_port: u16,
    pub p2p_port: u16,
    pub auth_port: u16,
    pub data_dir: String,
    pub jwt_secret_path: String,
}

impl Default for RethConfig {
    fn default() -> Self {
        Self {
            http_port: DEFAULT_HTTP_PORT,
            ws_port: DEFAULT_WS_PORT,
            p2p_port: DEFAULT_P2P_PORT,
            auth_port: DEFAULT_AUTH_PORT,
            data_dir: "/data".to_string(),
            jwt_secret_path: "/jwt/jwt.hex".to_string(),
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
        let docker = Docker::connect_with_local_defaults().map_err(|e| Error::Docker(e))?;
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

    async fn generate_jwt() -> crate::Result<String> {
        // Generate 32 random bytes for JWT
        let jwt_secret: [u8; 32] = rand::random();
        Ok(hex::encode(jwt_secret))
    }

    async fn store_jwt(&self, jwt: &str) -> crate::Result<()> {
        let container_id = self.container_id.lock().await;
        if let Some(id) = container_id.as_ref() {
            // Create exec instance to write JWT
            let exec = self
                .docker
                .create_exec(
                    id,
                    CreateExecOptions {
                        cmd: Some(vec![
                            "sh",
                            "-c",
                            &format!("echo {} > /etc/jwt/jwt.hex", jwt),
                        ]),
                        attach_stdout: Some(true),
                        attach_stderr: Some(true),
                        ..Default::default()
                    },
                )
                .await
                .map_err(Error::Docker)?;

            // Start exec instance
            if let StartExecResults::Attached { mut output, .. } = self
                .docker
                .start_exec(&exec.id, None)
                .await
                .map_err(Error::Docker)?
            {
                while let Some(Ok(output)) = output.next().await {
                    logging::debug!("JWT write output: {:?}", output);
                }
            }

            logging::info!("JWT secret stored successfully");
            Ok(())
        } else {
            Err(Error::Container("Container not created".into()))
        }
    }

    pub async fn create_container(&self) -> crate::Result<String> {
        // Create volumes
        let data_dir = "rethdata";
        let jwt_dir = "rethjwt";

        for volume in [data_dir, jwt_dir] {
            if let Err(_) = self.docker.inspect_volume(volume).await {
                self.docker
                    .create_volume(CreateVolumeOptions {
                        name: volume.to_string(),
                        ..Default::default()
                    })
                    .await
                    .map_err(Error::Docker)?;
            }
        }

        // Generate JWT
        let jwt = Self::generate_jwt().await?;

        let config = Config {
            image: Some(RETH_IMAGE.to_string()),
            entrypoint: Some(vec!["sh".into(), "-c".into()]),
            cmd: Some(vec![format!(
                "mkdir -p /etc/jwt && echo {} > /etc/jwt/jwt.hex && \
                     reth node \
                     --metrics=0.0.0.0:9001 \
                     --chain=mainnet \
                     --datadir=/root/.local/share/reth/mainnet \
                     --authrpc.jwtsecret=/etc/jwt/jwt.hex \
                     --authrpc.addr=0.0.0.0 \
                     --authrpc.port=8551 \
                     --http \
                     --http.addr=0.0.0.0 \
                     --http.port=8545",
                jwt
            )]),
            host_config: Some(HostConfig {
                binds: Some(vec![format!(
                    "{}:/root/.local/share/reth/mainnet",
                    data_dir
                )]),
                port_bindings: Some(HashMap::from([
                    (
                        "8551/tcp".into(),
                        Some(vec![PortBinding {
                            host_ip: Some("0.0.0.0".into()),
                            host_port: Some("8551".into()),
                        }]),
                    ),
                    (
                        "8545/tcp".into(),
                        Some(vec![PortBinding {
                            host_ip: Some("0.0.0.0".into()),
                            host_port: Some("8545".into()),
                        }]),
                    ),
                ])),
                ..Default::default()
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

    pub async fn check_health(&self) -> crate::Result<bool> {
        if let Some(id) = self.container_id.lock().await.as_ref() {
            let info = self
                .docker
                .inspect_container(id, None::<InspectContainerOptions>)
                .await
                .map_err(Error::Docker)?;

            logging::info!("Container info: {:?}", info);

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
                        logging::info!("Container log: {:?}", log);
                        if log.to_string().contains("error") || log.to_string().contains("Error") {
                            found_error = true;
                            logging::error!("Found error in logs: {:?}", log);
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
                .map(|r| r.map_err(Error::Docker).and_then(|l| Ok(l.to_string())));

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
