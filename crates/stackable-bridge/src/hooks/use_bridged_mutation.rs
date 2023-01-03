use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

use async_trait::async_trait;
use bounce::query::{use_mutation, UseMutationHandle};
use bounce::BounceStates;
use yew::prelude::*;

#[cfg(feature = "resolvable")]
use crate::resolvers::MutationResolver as BridgedMutation;
use crate::state::BridgeState;
#[cfg(not(feature = "resolvable"))]
use crate::types::BridgedMutation;
use crate::types::MutationResult;

struct MutationState<M>
where
    M: BridgedMutation,
{
    inner: MutationResult<M>,
}

impl<M> PartialEq for MutationState<M>
where
    M: BridgedMutation,
{
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

#[async_trait(?Send)]
impl<M> bounce::query::Mutation for MutationState<M>
where
    M: 'static + BridgedMutation,
{
    type Error = M::Error;
    type Input = M::Input;

    async fn run(
        states: &BounceStates,
        input: Rc<M::Input>,
    ) -> bounce::query::MutationResult<Self> {
        let bridge = states.get_atom_value::<BridgeState>();

        #[cfg(feature = "resolvable")]
        let mut meta = states
            .get_atom_value::<crate::state::BridgeMetadataState<M::Context>>()
            ._inner
            .as_ref()
            .map(|m| m.duplicate())
            .expect("failed to read the metadata, did you register your query / bridge?");
        #[cfg(not(feature = "resolvable"))]
        let mut meta = crate::BridgeMetadata::<()>::new();

        if let Some(token) = bridge.inner.read_token(states) {
            meta = meta.with_token(token.as_ref());
        }

        let connected = bridge
            .inner
            .clone()
            .connect(meta)
            .await
            .map_err(|m| M::into_mutation_error(m))?;

        Ok(Self {
            inner: connected.resolve_mutation::<M>(&input).await,
        }
        .into())
    }
}

/// A handle returned by [`use_bridged_mutation`].
///
/// This can be used to access the result or start the mutation.
pub struct UseBridgedMutationHandle<T>
where
    T: BridgedMutation + 'static,
{
    inner: UseMutationHandle<MutationState<T>>,
}

impl<T> UseBridgedMutationHandle<T>
where
    T: BridgedMutation + 'static,
{
    /// Runs a mutation with input.
    pub async fn run(&self, input: impl Into<Rc<T::Input>>) -> MutationResult<T> {
        self.inner.run(input).await?.inner.clone()
    }

    /// Returns the result of last finished mutation (if any).
    ///
    /// - `None` indicates that a mutation is currently loading or has yet to start(idling).
    /// - `Some(Ok(m))` indicates that the last mutation is successful and the content is stored in
    ///   `m`.
    /// - `Some(Err(e))` indicates that the last mutation has failed and the error is stored in `e`.
    pub fn result(&self) -> Option<&MutationResult<T>> {
        match self.inner.result()? {
            Ok(m) => Some(&m.inner),
            Err(_) => panic!("this can never happen!"),
        }
    }
}

impl<T> fmt::Debug for UseBridgedMutationHandle<T>
where
    T: BridgedMutation + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UseBridgedMutationHandle")
            .field("state", self.deref())
            .finish()
    }
}

impl<T> Clone for UseBridgedMutationHandle<T>
where
    T: BridgedMutation + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

/// Bridges a mutation.
#[hook]
pub fn use_bridged_mutation<T>() -> UseBridgedMutationHandle<T>
where
    T: 'static + BridgedMutation,
{
    let handle = use_mutation::<MutationState<T>>();

    UseBridgedMutationHandle { inner: handle }
}
