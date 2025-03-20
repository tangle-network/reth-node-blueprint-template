use blueprint_sdk::Job;
use blueprint_sdk::Router;
use blueprint_sdk::contexts::tangle::TangleClientContext;
use blueprint_sdk::crypto::sp_core::SpSr25519;
use blueprint_sdk::crypto::tangle_pair_signer::TanglePairSigner;
use blueprint_sdk::keystore::backends::Backend;
use blueprint_sdk::runner::BlueprintRunner;
use blueprint_sdk::runner::config::BlueprintEnvironment;
use blueprint_sdk::runner::tangle::config::TangleConfig;
use blueprint_sdk::tangle::consumer::TangleConsumer;
use blueprint_sdk::tangle::filters::MatchesServiceId;
use blueprint_sdk::tangle::layers::TangleLayer;
use blueprint_sdk::tangle::producer::TangleProducer;
use reth_docker_template_blueprint_lib::{
    RETH_START_JOB_ID, RETH_STOP_JOB_ID, RethConfig, RethContext, reth_start, reth_stop,
};
use std::path::PathBuf;
use tower::filter::FilterLayer;
use tracing::level_filters::LevelFilter;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), blueprint_sdk::Error> {
    setup_log();

    let env = BlueprintEnvironment::load()?;
    let sr25519_signer = env.keystore().first_local::<SpSr25519>()?;
    let sr25519_pair = env.keystore().get_secret::<SpSr25519>(&sr25519_signer)?;
    let st25519_signer = TanglePairSigner::new(sr25519_pair.0);

    let tangle_client = env.tangle_client().await?;
    let tangle_producer =
        TangleProducer::finalized_blocks(tangle_client.rpc_client.clone()).await?;
    let tangle_consumer = TangleConsumer::new(tangle_client.rpc_client.clone(), st25519_signer);

    let tangle_config = TangleConfig::default();

    // Create Reth context with proper configuration
    let reth_config = RethConfig {
        // Ensure we're using the correct path to the submodule
        submodule_path: PathBuf::from("local_reth"),
        block_tip: std::env::var("RETH_TIP").ok(),
        monitoring_port: 9000,
        grafana_port: 3000,
    };
    let reth_context = RethContext::new(reth_config.clone());

    // Log service URLs
    info!("Service URLs when Reth node is running:");
    info!(
        "Grafana dashboard: http://localhost:{}",
        reth_config.grafana_port
    );
    info!("Prometheus: http://localhost:9090");
    info!(
        "Metrics endpoint: http://localhost:{}",
        reth_config.monitoring_port
    );
    info!("");
    info!("Available job functions:");
    info!(
        "RETH_START_JOB_ID: {} - Start the Reth node",
        RETH_START_JOB_ID
    );
    info!(
        "RETH_STOP_JOB_ID: {} - Stop the Reth node",
        RETH_STOP_JOB_ID
    );

    let service_id = env.protocol_settings.tangle()?.service_id.unwrap();
    let result = BlueprintRunner::builder(tangle_config, env)
        .router(
            Router::new()
                // Add routes for state-changing operations only
                .route(RETH_START_JOB_ID, reth_start.layer(TangleLayer))
                .route(RETH_STOP_JOB_ID, reth_stop.layer(TangleLayer))
                // Add the service ID filter layer
                .layer(FilterLayer::new(MatchesServiceId(service_id)))
                // Set the Reth context
                .with_context(reth_context),
        )
        .producer(tangle_producer)
        .consumer(tangle_consumer)
        .with_shutdown_handler(async {
            info!("Shutting down Reth blueprint!");
            // Try to stop the Reth node on shutdown if it's running
            let context = RethContext::with_default_config();
            let status = reth_docker_template_blueprint_lib::monitoring::get_status(&context);
            if let Ok(status_str) = status {
                if !status_str.contains("No Reth services") {
                    info!("Attempting to stop Reth node...");
                    let _ = reth_docker_template_blueprint_lib::run_command(
                        &context,
                        "docker-compose",
                        &["down"],
                    );
                }
            }
        })
        .run()
        .await;

    if let Err(e) = result {
        error!("Runner failed! {e:?}");
    }

    Ok(())
}

pub fn setup_log() {
    use tracing_subscriber::util::SubscriberInitExt;

    let _ = tracing_subscriber::fmt::SubscriberBuilder::default()
        .without_time()
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::NONE)
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .finish()
        .try_init();
}
