//! Bridge resolvers.

use async_trait::async_trait;

use crate::types::{BridgedMutation, BridgedQuery, MutationResult, QueryResult};
use crate::BridgeMetadata;

/// The resolver of a bridge query.
///
/// This type is required to be implemented when the `resolvable` feature is enabled.
/// Please refer to the crate implementation for more information.
#[async_trait(?Send)]
pub trait QueryResolver: BridgedQuery {
    /// The context type.
    ///
    /// This type needs to match the `CTX` type parameter of the bridge it is added.
    type Context: 'static;

    /// Resolves the current query.
    async fn resolve(
        meta: &BridgeMetadata<Self::Context>,
        input: &Self::Input,
    ) -> QueryResult<Self>;
}

/// The resolver of a bridge mutation.
///
/// This type is required to be implemented when the `resolvable` feature is enabled.
/// Please refer to the crate implementation for more information.
#[async_trait(?Send)]
pub trait MutationResolver: BridgedMutation {
    /// The context type.
    ///
    /// This type needs to match the `CTX` type parameter of the bridge it is added.
    type Context: 'static;

    /// Resolves the current mutation.
    async fn resolve(
        meta: &BridgeMetadata<Self::Context>,
        input: &Self::Input,
    ) -> MutationResult<Self>;
}
