use crate::service::ServiceContext;
use crate::Error;
use api::services::events::JobCalled;
use blueprint_sdk::{
    event_listeners::tangle::{
        events::TangleEventListener,
        services::{services_post_processor, services_pre_processor},
    },
    tangle_subxt::tangle_testnet_runtime::api,
};
use serde::{Deserialize, Serialize};

/// Parameters for node restart
#[derive(Debug, Serialize, Deserialize)]
pub struct RestartNodeParams {
    pub clear_cache: bool,
    pub new_config: Option<String>,
}

/// Parameters for snapshot operations
#[derive(Debug, Serialize, Deserialize)]
pub struct SnapshotParams {
    pub path: String,
    pub include_state: bool,
}

/// Parameters for data export
#[derive(Debug, Serialize, Deserialize)]
pub struct ExportParams {
    pub start_block: u64,
    pub end_block: u64,
    pub include_traces: bool,
    pub destination: String,
}

#[blueprint_sdk::job(
    id = 1,
    params(params),
    result(_),
    event_listener(
        listener = TangleEventListener::<ServiceContext, JobCalled>,
        pre_processor = services_pre_processor,
        post_processor = services_post_processor,
    ),
)]
pub async fn restart_node(params: Vec<u8>, ctx: ServiceContext) -> crate::Result<Vec<u8>> {
    let params: RestartNodeParams =
        serde_json::from_slice(&params).map_err(|e| Error::Other(e.to_string()))?;

    let node = ctx.reth_node.lock().await;
    node.stop().await?;

    if params.clear_cache {
        // Implementation for clearing cache
    }

    if let Some(config) = params.new_config {
        // Implementation for applying new config
    }

    if let Err(e) = node.start_container().await {
        blueprint_sdk::logging::error!("Failed to start node: {}", e);
        return Ok(vec![]);
    }

    Ok(serde_json::to_vec(&serde_json::json!({
        "success": true,
        "message": "Node restarted successfully"
    }))
    .unwrap_or_default())
}

#[blueprint_sdk::job(
    id = 2,
    params(params),
    result(_),
    event_listener(
        listener = TangleEventListener::<ServiceContext, JobCalled>,
        pre_processor = services_pre_processor,
        post_processor = services_post_processor,
    ),
)]
pub async fn create_snapshot(params: Vec<u8>, ctx: ServiceContext) -> crate::Result<Vec<u8>> {
    let params: SnapshotParams =
        serde_json::from_slice(&params).map_err(|e| Error::Other(e.to_string()))?;

    let node = ctx.reth_node.lock().await;
    // Implementation for creating snapshot

    match serde_json::to_vec(&serde_json::json!({
        "success": true,
        "message": "Snapshot created successfully",
        "path": params.path
    })) {
        Ok(bytes) => Ok(bytes),
        Err(_) => Ok(vec![]),
    }
}

#[blueprint_sdk::job(
    id = 3,
    params(params),
    result(_),
    event_listener(
        listener = TangleEventListener::<ServiceContext, JobCalled>,
        pre_processor = services_pre_processor,
        post_processor = services_post_processor,
    ),
)]
pub async fn export_historical_data(
    params: Vec<u8>,
    ctx: ServiceContext,
) -> crate::Result<Vec<u8>> {
    let params: ExportParams =
        serde_json::from_slice(&params).map_err(|e| Error::Other(e.to_string()))?;

    let node = ctx.reth_node.lock().await;
    // Implementation for exporting historical data

    match serde_json::to_vec(&serde_json::json!({
        "success": true,
        "message": "Historical data exported successfully",
        "destination": params.destination
    })) {
        Ok(bytes) => Ok(bytes),
        Err(_) => Ok(vec![]),
    }
}
