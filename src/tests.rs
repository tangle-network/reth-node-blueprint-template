use crate::{
    jobs::RestartNodeParams,
    reth::{RethConfig, RethNode},
    service::ServiceContext,
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
use color_eyre::Result;
use futures::StreamExt;

async fn setup_test_env(reth_node: RethNode) -> Result<(TangleTestHarness, ServiceContext)> {
    setup_log();
    let temp_dir = tempfile::TempDir::new()?;
    let harness = TangleTestHarness::setup(temp_dir).await?;

    let context = ServiceContext::new(StdGadgetConfiguration::default(), reth_node);
    Ok((harness, context))
}

#[tokio::test]
async fn test_background_service() -> Result<()> {
    println!("Starting background service test");
    let reth_config = RethConfig::default();
    println!("Creating new RethNode with default config");
    let reth_node = crate::reth::RethNode::new(reth_config).await?;

    println!("Setting up test environment");
    let (harness, _context) = setup_test_env(reth_node.clone()).await?;
    println!("Setting up services");
    let (mut test_env, _service_id) = harness.setup_services().await?;
    println!("Adding background service");
    test_env.add_background_service(reth_node.clone());
    println!("Running test environment");
    test_env.run_runner().await?;

    println!("Waiting for node to become healthy");
    reth_node.wait_for_healthy().await?;
    let health = reth_node.check_health().await?;
    println!("Node health check result: {}", health);
    assert!(health, "Node should be healthy");

    // Get and print logs
    println!("Fetching node logs");
    let mut logs = reth_node.get_logs().await?;
    while let Some(log) = logs.next().await {
        match log {
            Ok(log) => {
                println!("Log: {}", log);
                if log.contains("Node started") {
                    println!("Found node started message!");
                }
                if log.contains("Syncing") {
                    println!("Node is syncing!");
                }
            }
            Err(e) => println!("Error reading log: {}", e),
        }
    }

    println!("Background service test completed successfully");
    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_restart_node() -> Result<()> {
    println!("Starting restart node test");
    let reth_config = RethConfig::default();
    println!("Creating new RethNode with default config");
    let reth_node = crate::reth::RethNode::new(reth_config).await?;

    println!("Setting up test environment");
    let (harness, _context) = setup_test_env(reth_node.clone()).await?;
    println!("Setting up services");
    let (mut test_env, service_id) = harness.setup_services().await?;
    println!("Adding background service");
    test_env.add_background_service(reth_node);

    println!("Spawning test environment runner");
    tokio::spawn(async move {
        test_env.run_runner().await.unwrap();
    });

    println!("Preparing restart node parameters");
    let params = RestartNodeParams {
        clear_cache: false,
        new_config: None,
    };
    let input_bytes = serde_json::to_vec(&params)?;
    let input_bytes_fields = input_bytes.iter().map(|v| InputValue::Uint8(*v)).collect();

    println!("Executing restart job");
    let _ = harness
        .execute_job(
            service_id,
            1,
            vec![InputValue::List(BoundedVec(input_bytes_fields))],
            vec![OutputValue::Bool(true)],
        )
        .await?;

    println!("Restart node test completed successfully");
    Ok(())
}

#[tokio::test]
async fn test_node_lifecycle() -> Result<()> {
    println!("Starting node lifecycle test");
    let reth_config = RethConfig::default();
    let mut node = crate::reth::RethNode::new(reth_config).await?;

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
    Ok(())
}
