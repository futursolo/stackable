use std::any::TypeId;

use thiserror::Error;

/// The bridge error type.
#[derive(Error, Debug)]
pub enum BridgeError {
    /// Some network error happened while communicating with the backend.
    #[error("failed to communicate with server")]
    Network(#[from] gloo_net::Error),

    /// The bridge failed to encode / decode the message from the other side.
    #[error("failed to encode / decode content")]
    Encoding(#[from] bincode::Error),

    /// The type does not have a valid index.
    #[error("failed to find type with index: {}", .0)]
    InvalidIndex(usize),

    /// The type is not valid.
    #[error("failed to find type: {:?}", .0)]
    InvalidType(TypeId),
}

/// The bridge result type.
pub type BridgeResult<T> = Result<T, BridgeError>;
