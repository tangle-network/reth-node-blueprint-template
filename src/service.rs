use crate::reth::RethNode;
use blueprint_sdk::config::StdGadgetConfiguration;
use blueprint_sdk::macros::contexts::{ServicesContext, TangleClientContext};
use blueprint_sdk::std::sync::Arc;
use blueprint_sdk::tokio::sync::Mutex;

#[derive(Clone, TangleClientContext, ServicesContext)]
pub struct ServiceContext {
    #[config]
    pub config: StdGadgetConfiguration,
    #[call_id]
    pub call_id: Option<u64>,
    pub reth_node: Arc<Mutex<RethNode>>,
}

impl ServiceContext {
    pub fn new(config: StdGadgetConfiguration, reth_node: RethNode) -> Self {
        Self {
            config,
            reth_node: Arc::new(Mutex::new(reth_node)),
            call_id: None,
        }
    }
}
