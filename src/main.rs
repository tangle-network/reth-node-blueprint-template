use blueprint_sdk::runners::{core::runner::BlueprintRunner, tangle::tangle::TangleConfig};
use bollard;
use color_eyre::Result;
use reth_rpc_blueprint_template as blueprint;

#[blueprint_sdk::main(env)]
async fn main() -> Result<()> {
    let docker = bollard::Docker::connect_with_local_defaults()?;

    // Generate JWT and initialize environment
    let jwt_config = blueprint::JwtConfig::new()?;
    blueprint::initialize_environment(&docker, &jwt_config).await?;

    // Create nodes with default configs
    let reth_config = blueprint::reth::RethConfig::default();
    let reth_node = blueprint::reth::RethNode::new(reth_config).await?;

    let nimbus_config = blueprint::nimbus::NimbusConfig::default();
    let nimbus_node = blueprint::nimbus::NimbusNode::new(nimbus_config).await?;

    // Create service context
    let context = blueprint::service::ServiceContext::new(env.clone(), reth_node.clone());

    blueprint_sdk::logging::info!("Starting the event watcher ...");
    let tangle_config = TangleConfig::default();
    BlueprintRunner::new(tangle_config, env)
        .background_service(Box::new(reth_node))
        .background_service(Box::new(nimbus_node))
        .run()
        .await?;

    blueprint_sdk::logging::info!("Exiting...");
    Ok(())
}
