//! Bridge between the frontend and backend.
//!
//! This module is a RPC implementation that facilitates communications between frontend and
//! backend.
//!
//! It supports the following types:
//!
//! - [Query](types::BridgedQuery)
//! - [Mutation](types::BridgedMutation)
//!
//! Bridge has 2 connection methods `local` and `remote`. When the `resolvable` feature is
//! enabled, bridges will be conntected with the local method and can process requests with
//! resolvers. This can be used for server-side rendering and processing requests from a bridge
//! connected with the remote method. If the `resolvable` feature is disabled, it will send the
//! request to the bridge endpoint which will process the bridge at the server-side. This is usually
//! used for client-side rendering.
//!
//! When the `resolvable` feature is enabled, it will require query and mutations to implement their
//! resolver type which is responsible for resolving the request.
//!
//! You can check out the [example](https://github.com/futursolo/stackable/blob/master/examples/fullstack/api/src/lib.rs) for how to implement resolvers.

#![deny(clippy::all)]
#![deny(missing_debug_implementations)]

mod bridge;
mod error;
pub mod hooks;
#[cfg(feature = "resolvable")]
pub mod resolvers;
pub mod state;
pub mod types;

pub use bridge::{Bridge, BridgeBuilder, BridgeMetadata, ConnectedBridge};
pub use error::{BridgeError, BridgeResult};
