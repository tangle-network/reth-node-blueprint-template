use crate::{
    jobs::RestartNodeParams,
    nimbus::{NimbusConfig, NimbusNode},
    reth::{RethConfig, RethNode},
    service::ServiceContext,
    JwtConfig,
};
use blueprint_sdk::{
    config::StdGadgetConfiguration,
    logging::setup_log,
    testing::{
        tempfile,
        utils::{
            harness::TestHarness,
            runner::TestEnv,
            tangle::{blueprint_serde::BoundedVec, InputValue, OutputValue, TangleTestHarness},
        },
    },
    tokio,
};
use bollard::Docker;
use color_eyre::Result;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::Mutex;

async fn setup_test_env(reth_node: RethNode) -> Result<(TangleTestHarness, ServiceContext)> {
    setup_log();
    let temp_dir = tempfile::TempDir::new()?;
    let harness = TangleTestHarness::setup(temp_dir).await?;

    let context = ServiceContext::new(StdGadgetConfiguration::default(), reth_node);
    Ok((harness, context))
}

#[derive(Clone)]
struct TestContext {
    nodes: Arc<Mutex<Vec<(RethNode, NimbusNode)>>>,
    docker: Arc<Docker>,
}

impl TestContext {
    async fn new() -> Result<Self> {
        let docker = Docker::connect_with_local_defaults()?;
        Ok(Self {
            nodes: Arc::new(Mutex::new(Vec::new())),
            docker: Arc::new(docker),
        })
    }

    async fn add_node_pair(&self, reth: RethNode, nimbus: NimbusNode) {
        self.nodes.lock().await.push((reth, nimbus));
    }

    async fn cleanup(&self) -> Result<()> {
        for (reth, nimbus) in self.nodes.lock().await.iter() {
            reth.cleanup().await?;
            nimbus.cleanup().await?;
        }

        // Cleanup shared resources
        for volume in ["reth_data", "nimbus_data", "reth_jwt"] {
            if let Ok(_) = self.docker.inspect_volume(volume).await {
                let _ = self.docker.remove_volume(volume, None).await;
            }
        }

        if let Ok(_) = self
            .docker
            .inspect_network::<String>("eth_network", None)
            .await
        {
            let _ = self.docker.remove_network("eth_network").await;
        }

        Ok(())
    }
}

#[tokio::test]
async fn test_background_service() -> Result<()> {
    let test_ctx = TestContext::new().await?;

    // Setup Ctrl+C handler
    let test_ctx_clone = test_ctx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        println!("Received Ctrl+C, cleaning up...");
        test_ctx_clone.cleanup().await.unwrap();
        std::process::exit(0);
    });

    // Initialize environment
    let jwt_config = JwtConfig::new()?;
    crate::initialize_environment(&test_ctx.docker, &jwt_config).await?;

    // Initialize both nodes
    let reth_node = RethNode::new(RethConfig::default()).await?;
    let nimbus_node = NimbusNode::new(NimbusConfig::default()).await?;
    test_ctx
        .add_node_pair(reth_node.clone(), nimbus_node.clone())
        .await;

    // Setup test environment with both nodes
    let (harness, _context) = setup_test_env(reth_node.clone()).await?;
    let (mut test_env, _service_id) = harness.setup_services().await?;

    println!("Starting background service test");
    println!("Setting up test environment");
    println!("Setting up services");
    println!("Adding background services");

    // Add both services
    test_env.add_background_service(reth_node.clone());
    test_env.add_background_service(nimbus_node.clone());

    println!("Running test environment");
    test_env.run_runner().await?;

    // Wait for both nodes to become healthy
    println!("Waiting for nodes to become healthy");
    reth_node.wait_for_healthy().await?;
    nimbus_node.wait_for_healthy().await?;

    let reth_health = reth_node.check_health().await?;
    let nimbus_health = nimbus_node.check_health().await?;

    println!("RETH node health check result: {}", reth_health);
    println!("Nimbus node health check result: {}", nimbus_health);

    assert!(reth_health, "RETH node should be healthy");
    assert!(nimbus_health, "Nimbus node should be healthy");

    // Test inter-node communication
    println!("Testing inter-node communication");
    let mut reth_logs = reth_node.get_logs().await?;
    let mut nimbus_logs = nimbus_node.get_logs().await?;

    let mut found_connection = false;
    while let Some(log) = reth_logs.next().await {
        match log {
            Ok(log) => {
                if log.contains("Connected to Nimbus") || log.contains("consensus client connected")
                {
                    found_connection = true;
                    println!("Found connection confirmation in RETH logs!");
                    break;
                }
            }
            Err(e) => println!("Error reading RETH log: {}", e),
        }
    }

    let mut found_sync = false;
    while let Some(log) = nimbus_logs.next().await {
        match log {
            Ok(log) => {
                if log.contains("Connected to execution client") || log.contains("Syncing") {
                    found_sync = true;
                    println!("Found sync confirmation in Nimbus logs!");
                    break;
                }
            }
            Err(e) => println!("Error reading Nimbus log: {}", e),
        }
    }

    assert!(found_connection, "Nodes should establish connection");
    assert!(found_sync, "Nodes should start syncing");

    println!("Background service test completed successfully");
    test_ctx.cleanup().await?;
    Ok(())
}

#[tokio::test]
async fn test_node_lifecycle() -> Result<()> {
    let test_ctx = TestContext::new().await?;

    // Initialize environment
    let jwt_config = JwtConfig::new()?;
    crate::initialize_environment(&test_ctx.docker, &jwt_config).await?;

    println!("Starting node lifecycle test");
    let reth_config = RethConfig::default();
    let mut node = RethNode::new(reth_config).await?;

    // Initialize and start the node
    println!("Initializing and starting node");
    node.initialize().await?;
    node.start_container().await?;
    node.wait_for_healthy().await?;
    let health = node.check_health().await?;
    println!("Initial node health check: {}", health);
    assert!(health);

    // Stop the node
    println!("Stopping node");
    node.stop().await?;
    let health = node.check_health().await?;
    println!("Node health check after stop: {}", health);
    assert!(!health);

    // Start the node again
    println!("Restarting node");
    node.start_container().await?;
    node.wait_for_healthy().await?;
    let health = node.check_health().await?;
    println!("Node health check after restart: {}", health);
    assert!(health);

    // Remove the node
    println!("Removing node");
    node.remove().await?;
    let health = node.check_health().await?;
    println!("Final node health check: {}", health);
    assert!(!health);

    println!("Node lifecycle test completed successfully");
    test_ctx.cleanup().await?;
    Ok(())
}

#[tokio::test]
async fn test_jwt_sharing() -> Result<()> {
    let test_ctx = TestContext::new().await?;

    // Initialize environment with JWT
    let jwt_config = JwtConfig::new()?;
    crate::initialize_environment(&test_ctx.docker, &jwt_config).await?;

    // Create and start both nodes
    let mut reth_node = RethNode::new(RethConfig::default()).await?;
    let mut nimbus_node = NimbusNode::new(NimbusConfig::default()).await?;

    // Initialize and start RETH
    reth_node.initialize().await?;
    reth_node.start_container().await?;
    reth_node.wait_for_healthy().await?;

    // Initialize and start Nimbus
    nimbus_node.initialize().await?;
    nimbus_node.start_container().await?;
    nimbus_node.wait_for_healthy().await?;

    // Wait for connection establishment
    let mut reth_logs = reth_node.get_logs().await?;
    let mut nimbus_logs = nimbus_node.get_logs().await?;

    let mut jwt_auth_successful = false;
    while let Some(log) = reth_logs.next().await {
        match log {
            Ok(log) => {
                if log.contains("JWT authentication successful")
                    || log.contains("consensus client connected")
                {
                    jwt_auth_successful = true;
                    break;
                }
            }
            Err(e) => println!("Error reading RETH log: {}", e),
        }
    }

    let mut consensus_connected = false;
    while let Some(log) = nimbus_logs.next().await {
        match log {
            Ok(log) => {
                if log.contains("Connected to execution client") {
                    consensus_connected = true;
                    break;
                }
            }
            Err(e) => println!("Error reading Nimbus log: {}", e),
        }
    }

    assert!(jwt_auth_successful, "JWT authentication should succeed");
    assert!(consensus_connected, "Consensus client should connect");

    // Cleanup
    reth_node.cleanup().await?;
    nimbus_node.cleanup().await?;
    test_ctx.cleanup().await?;

    Ok(())
}
