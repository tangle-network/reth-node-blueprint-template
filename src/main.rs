use blueprint_sdk::runners::{core::runner::BlueprintRunner, tangle::tangle::TangleConfig};
use color_eyre::Result;
use reth_rpc_blueprint_template as blueprint;

#[blueprint_sdk::main(env)]
async fn main() -> Result<()> {
    // Create a new RETH node with default configuration
    let reth_config = blueprint::reth::RethConfig::default();
    let reth_node = blueprint::reth::RethNode::new(reth_config).await?;

    // Create service context with the RETH node
    let context = blueprint::service::ServiceContext::new(env.clone(), reth_node.clone());

    blueprint_sdk::logging::info!("Starting the event watcher ...");
    let tangle_config = TangleConfig::default();
    BlueprintRunner::new(tangle_config, env)
        .background_service(Box::new(reth_node))
        .run()
        .await?;

    blueprint_sdk::logging::info!("Exiting...");
    Ok(())
}
