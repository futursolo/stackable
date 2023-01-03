use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

use async_trait::async_trait;
use bounce::query::{use_prepared_query, UseQueryHandle};
use bounce::BounceStates;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use yew::prelude::*;
use yew::suspense::SuspensionResult;

#[cfg(feature = "resolvable")]
use crate::resolvers::QueryResolver as BridgedQuery;
use crate::state::BridgeState;
#[cfg(not(feature = "resolvable"))]
use crate::types::BridgedQuery;
use crate::types::QueryResult;

#[derive(Debug, PartialEq)]
struct QueryState<Q>
where
    Q: BridgedQuery,
{
    inner: QueryResult<Q>,
}

impl<Q> Clone for QueryState<Q>
where
    Q: BridgedQuery,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<Q> Eq for QueryState<Q> where Q: BridgedQuery + Eq {}

impl<Q> Serialize for QueryState<Q>
where
    Q: BridgedQuery,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.inner.as_deref().serialize(serializer)
    }
}

impl<'de, Q> Deserialize<'de> for QueryState<Q>
where
    Q: BridgedQuery,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self {
            inner: std::result::Result::<Q, Q::Error>::deserialize(deserializer)?.map(Rc::new),
        })
    }
}
#[async_trait(?Send)]
impl<Q> bounce::query::Query for QueryState<Q>
where
    Q: 'static + BridgedQuery,
{
    type Error = Q::Error;
    type Input = Q::Input;

    async fn query(
        states: &BounceStates,
        input: Rc<Self::Input>,
    ) -> bounce::query::QueryResult<Self> {
        let bridge = states.get_atom_value::<BridgeState>();

        #[cfg(feature = "resolvable")]
        let mut meta = states
            .get_atom_value::<crate::state::BridgeMetadataState<Q::Context>>()
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
            .map_err(|m| Q::into_query_error(m))?;

        Ok(Self {
            inner: connected.resolve_query::<Q>(&input).await,
        }
        .into())
    }
}

/// A handle returned by [`use_bridged_query`].
///
/// This type dereferences to [`QueryResult<T>`].
pub struct UseBridgedQueryHandle<T>
where
    T: BridgedQuery + 'static,
{
    inner: UseQueryHandle<QueryState<T>>,
}

impl<T> UseBridgedQueryHandle<T>
where
    T: BridgedQuery + 'static,
{
    /// Refreshes the query.
    ///
    /// The query will be refreshed with the input provided to the hook.
    pub async fn refresh(&self) -> QueryResult<T> {
        self.inner.refresh().await?.inner.clone()
    }
}

impl<T> Clone for UseBridgedQueryHandle<T>
where
    T: BridgedQuery + 'static,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Deref for UseBridgedQueryHandle<T>
where
    T: BridgedQuery + 'static,
{
    type Target = QueryResult<T>;

    fn deref(&self) -> &Self::Target {
        match self.inner.deref() {
            Ok(ref m) => &m.inner,
            _ => panic!("this variant can never happen!"),
        }
    }
}

impl<T> fmt::Debug for UseBridgedQueryHandle<T>
where
    T: BridgedQuery + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UseQueryHandle")
            .field("value", self.deref())
            .finish()
    }
}

/// Bridges a query.
#[hook]
pub fn use_bridged_query<Q>(input: Rc<Q::Input>) -> SuspensionResult<UseBridgedQueryHandle<Q>>
where
    Q: 'static + BridgedQuery,
{
    let handle = use_prepared_query::<QueryState<Q>>(input)?;

    Ok(UseBridgedQueryHandle { inner: handle })
}
