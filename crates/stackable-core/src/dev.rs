//! Stackable development server utilities.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Development server metadata.
///
/// This information is passed from stackctl to the server when it is started as a development
/// server.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StackctlMetadata {
    pub listen_addr: String,
    pub frontend_dev_build_dir: PathBuf,
}

impl StackctlMetadata {
    /// The environment variable used by metadata.
    pub const ENV_NAME: &str = "STACKCTL_METADATA";

    /// Parses the metadata from a json string.
    pub fn from_json(s: &str) -> serde_json::Result<Self> {
        serde_json::from_str(s)
    }

    /// Serialises the metadata to a json string.
    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}
