pub mod jobs;
pub mod reth;
pub mod service;

#[cfg(test)]
mod tests;

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
}

pub type Result<T> = blueprint_sdk::std::result::Result<T, Error>;
