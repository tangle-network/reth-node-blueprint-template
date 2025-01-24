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
    image::CreateImageOptions,
    models::HostConfig,
    Docker,
};
use futures::StreamExt;

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
            ]),
            host_config: Some(HostConfig {
                binds: Some(vec![
                    format!("reth_data:{}", self.config.data_dir),
                    format!("reth_jwt:{}", self.config.jwt_secret_path),
                ]),
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

            // Check container is running
            if !info.state.and_then(|s| s.running).unwrap_or(false) {
                logging::warn!("RETH container is not running");
                return Ok(false);
            }

            // Check logs for readiness
            if let Ok(mut logs) = self.get_logs().await {
                while let Some(log) = logs.next().await {
                    match log {
                        Ok(log) if log.contains("Node started") => {
                            logging::info!("RETH node is ready");
                            return Ok(true);
                        }
                        Ok(_) => continue,
                        Err(e) => {
                            logging::error!("Error reading logs: {}", e);
                            return Ok(false);
                        }
                    }
                }
            }
        }
        Ok(false)
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
        let (tx, rx) = oneshot::channel();
        let mut node = self.clone();

        tokio::spawn(async move {
            let result = async {
                // Initialize if needed
                node.initialize().await?;

                // Start container
                node.start_container().await?;

                // Wait for healthy
                node.wait_for_healthy().await?;

                // Start background monitoring
                node.monitor_health().await
            }
            .await;

            let _ = tx.send(result.map_err(|e| RunnerError::Other(e.to_string())));
        });

        Ok(rx)
    }
}
