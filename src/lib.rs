pub mod jobs;
pub mod lighthouse;
pub mod nimbus;
pub mod reth;
pub mod service;

#[cfg(test)]
mod tests;

use blueprint_sdk::logging;
use blueprint_sdk::std::collections::HashMap;
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions, StartContainerOptions,
};
use bollard::network::CreateNetworkOptions;
use bollard::secret::HostConfig;
use bollard::volume::CreateVolumeOptions;
use bollard::Docker;
use futures::StreamExt;
use hex;
use rand;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Docker error: {0}")]
    Docker(#[from] bollard::errors::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Container error: {0}")]
    Container(String),

    #[error("Other error: {0}")]
    Other(String),

    #[error("JWT error: {0}")]
    Jwt(String),
}

pub type Result<T> = blueprint_sdk::std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct JwtConfig {
    pub secret: String,
}

impl JwtConfig {
    pub fn new() -> crate::Result<Self> {
        let secret: [u8; 32] = rand::random();
        let secret = hex::encode(secret);
        Ok(Self { secret })
    }
}

pub async fn setup_jwt(docker: &Docker, jwt: &str) -> Result<()> {
    logging::info!("Setting up JWT with secret: {}", jwt);

    // Create volume if it doesn't exist
    if let Err(_) = docker.inspect_volume("reth_jwt").await {
        logging::info!("Creating JWT volume");
        docker
            .create_volume(CreateVolumeOptions {
                name: "reth_jwt".to_string(),
                ..Default::default()
            })
            .await
            .map_err(Error::Docker)?;
    }

    // Create temporary container to write JWT and verify
    let config = Config {
        image: Some("alpine:latest".to_string()),
        cmd: Some(vec![
            "sh".to_string(),
            "-c".to_string(),
            format!(
                "set -ex && \
                 mkdir -p /etc/jwt && \
                 echo {} > /etc/jwt/jwt.hex && \
                 chmod 644 /etc/jwt/jwt.hex && \
                 ls -la /etc/jwt && \
                 cat /etc/jwt/jwt.hex", // Verify the file exists and has content
                jwt
            ),
        ]),
        host_config: Some(HostConfig {
            binds: Some(vec!["reth_jwt:/etc/jwt".into()]),
            ..Default::default()
        }),
        ..Default::default()
    };

    logging::info!("Creating temporary container to write JWT");
    let container = docker
        .create_container(None::<CreateContainerOptions<String>>, config)
        .await
        .map_err(Error::Docker)?;

    // Get logs to see what's happening
    let mut logs = docker.logs(
        &container.id,
        Some(LogsOptions::<String> {
            stdout: true,
            stderr: true,
            follow: true,
            timestamps: true,
            ..Default::default()
        }),
    );

    // Start container
    logging::info!("Starting temporary container");
    docker
        .start_container(&container.id, None::<StartContainerOptions<String>>)
        .await
        .map_err(Error::Docker)?;

    // Collect logs while waiting
    while let Some(log) = logs.next().await {
        match log {
            Ok(log) => logging::info!("JWT setup log: {:?}", log),
            Err(e) => logging::error!("Error reading JWT setup log: {}", e),
        }
    }

    // Wait for container to finish
    logging::info!("Waiting for JWT setup to complete");
    let mut wait_stream = docker.wait_container::<String>(&container.id, None);
    while let Some(exit) = wait_stream.next().await {
        match exit {
            Ok(exit) => {
                if exit.status_code != 0 {
                    return Err(Error::Container(format!(
                        "JWT setup container exited with code {}",
                        exit.status_code
                    )));
                }
                logging::info!("JWT setup completed successfully");
                break;
            }
            Err(e) => return Err(Error::Docker(e)),
        }
    }

    // Cleanup temporary container
    logging::info!("Cleaning up temporary container");
    docker
        .remove_container(
            &container.id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await
        .map_err(Error::Docker)?;

    Ok(())
}

pub async fn setup_network(docker: &Docker) -> Result<()> {
    let network_name = "eth_network";

    // Create network if it doesn't exist
    if let Err(_) = docker.inspect_network::<String>(network_name, None).await {
        docker
            .create_network(CreateNetworkOptions {
                name: network_name.to_string(),
                driver: "bridge".to_string(),
                ..Default::default()
            })
            .await
            .map_err(Error::Docker)?;
        logging::info!("Created Docker network: {}", network_name);
    }
    Ok(())
}

pub async fn setup_volumes(docker: &Docker) -> Result<()> {
    // Create required volumes if they don't exist
    for volume in ["reth_data", "reth_jwt"] {
        if let Err(_) = docker.inspect_volume(volume).await {
            docker
                .create_volume(CreateVolumeOptions {
                    name: volume.to_string(),
                    driver_opts: HashMap::from([
                        ("type".to_string(), "none".to_string()),
                        ("o".to_string(), "bind,rw,mode=700".to_string()),
                    ]),
                    ..Default::default()
                })
                .await
                .map_err(Error::Docker)?;
            logging::info!("Created volume: {}", volume);
        }
    }
    Ok(())
}

pub async fn initialize_environment(docker: &Docker, jwt: &JwtConfig) -> Result<()> {
    // Set up network and volumes
    setup_network(docker).await?;
    setup_volumes(docker).await?;

    // Initialize JWT
    setup_jwt(docker, &jwt.secret).await?;

    Ok(())
}
