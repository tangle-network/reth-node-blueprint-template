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
    let reth_config = RethConfig::default();
    let reth_node = crate::reth::RethNode::new(reth_config).await?;

    let (harness, _context) = setup_test_env(reth_node.clone()).await?;
    let (mut test_env, _service_id) = harness.setup_services().await?;
    test_env.add_background_service(reth_node.clone());

    // Spawn the test environment runner
    tokio::spawn(async move {
        test_env.run_runner().await.unwrap();
    });

    // Wait for node to be healthy
    reth_node.wait_for_healthy().await?;
    assert!(reth_node.check_health().await?, "Node should be healthy");

    // Get and print logs
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

    Ok(())
}

#[tokio::test]
#[ignore]
async fn test_restart_node() -> Result<()> {
    let reth_config = RethConfig::default();
    let reth_node = crate::reth::RethNode::new(reth_config).await?;

    let (harness, _context) = setup_test_env(reth_node.clone()).await?;
    let (mut test_env, service_id) = harness.setup_services().await?;
    test_env.add_background_service(reth_node);

    tokio::spawn(async move {
        test_env.run_runner().await.unwrap();
    });

    let params = RestartNodeParams {
        clear_cache: false,
        new_config: None,
    };
    let input_bytes = serde_json::to_vec(&params)?;
    let input_bytes_fields = input_bytes.iter().map(|v| InputValue::Uint8(*v)).collect();

    let _ = harness
        .execute_job(
            service_id,
            1,
            vec![InputValue::List(BoundedVec(input_bytes_fields))],
            vec![OutputValue::Bool(true)],
        )
        .await?;

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
