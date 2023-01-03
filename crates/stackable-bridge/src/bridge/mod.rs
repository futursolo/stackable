use std::any::TypeId;
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bounce::Selector;
use serde::{Deserialize, Serialize};

use crate::error::{BridgeError, BridgeResult};
use crate::types::{MutationResult, QueryResult};

mod metadata;

pub use metadata::BridgeMetadata;

#[derive(Debug, Serialize, Deserialize)]
struct Incoming<'a> {
    query_index: usize,
    input: &'a [u8],
}

/// The Bridge Builder.
#[derive(Default)]
pub struct BridgeBuilder {
    #[cfg(feature = "resolvable")]
    ctx_id: Option<TypeId>,
    #[cfg(feature = "resolvable")]
    resolvers: Resolvers,
    #[cfg(not(feature = "resolvable"))]
    query_ids: Vec<TypeId>,
    #[cfg(not(feature = "resolvable"))]
    read_token: Option<ReadToken>,
}

impl BridgeBuilder {
    /// Adds a query.
    pub fn add_query<T>(self) -> Self
    where
        T: 'static + BridgedQuery,
    {
        self.add_query_impl::<T>()
    }

    /// Adds a mutation.
    pub fn add_mutation<T>(self) -> Self
    where
        T: 'static + BridgedMutation,
    {
        self.add_mutation_impl::<T>()
    }

    /// Selects the token from a bounce state.
    pub fn with_token_selector<T>(self) -> Self
    where
        T: 'static + Selector + AsRef<str>,
    {
        self.with_token_selector_impl::<T>()
    }

    /// Creates a bridge.
    pub fn build(self) -> Bridge {
        static ID: AtomicUsize = AtomicUsize::new(0);
        let id = ID.fetch_add(1, Ordering::AcqRel);

        Bridge {
            inner: self.into(),
            id,
        }
    }
}

impl fmt::Debug for BridgeBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("BridgeBuilder")
    }
}

#[cfg(feature = "resolvable")]
mod feat_resolvable {
    use std::any::Any;
    use std::rc::Rc;
    use std::sync::Arc;

    use bounce::BounceStates;
    use futures::future::LocalBoxFuture;
    use futures::FutureExt;

    use super::*;
    pub(super) use crate::resolvers::{
        MutationResolver as BridgedMutation, QueryResolver as BridgedQuery,
    };

    pub(super) type Resolvers = Vec<
        Arc<
            dyn Send + Sync + Fn(&dyn Any, &[u8]) -> LocalBoxFuture<'static, BridgeResult<Vec<u8>>>,
        >,
    >;

    impl Bridge {
        pub(crate) fn read_token(&self, _states: &BounceStates) -> Option<Rc<dyn AsRef<str>>> {
            None
        }
    }

    impl<CTX> ConnectedBridge<CTX>
    where
        CTX: 'static,
    {
        pub(crate) async fn resolve_query<T>(&self, input: &T::Input) -> QueryResult<T>
        where
            T: 'static + BridgedQuery<Context = CTX>,
        {
            T::resolve(&self.metadata, input).await
        }

        pub(crate) async fn resolve_mutation<T>(&self, input: &T::Input) -> MutationResult<T>
        where
            T: 'static + BridgedMutation<Context = CTX>,
        {
            T::resolve(&self.metadata, input).await
        }

        pub async fn resolve_encoded(&self, incoming: &[u8]) -> BridgeResult<Vec<u8>> {
            let incoming: Incoming<'_> = bincode::deserialize(incoming)?;

            let resolver = self
                .inner
                .inner
                .resolvers
                .get(incoming.query_index)
                .ok_or(BridgeError::InvalidIndex(incoming.query_index))?;

            resolver(&self.metadata, incoming.input).await
        }
    }

    impl BridgeBuilder {
        pub(super) fn add_query_impl<T>(mut self) -> Self
        where
            T: 'static + BridgedQuery,
        {
            let ctx_id = TypeId::of::<T::Context>();
            if let Some(m) = self.ctx_id {
                assert_eq!(
                    m, ctx_id,
                    "all resolvers should have the same context type!"
                );
            }
            self.ctx_id = Some(ctx_id);

            let resolver = Arc::new(|metadata: &dyn Any, input: &[u8]| {
                let input = match bincode::deserialize::<T::Input>(input) {
                    Ok(m) => m,
                    Err(e) => return async move { Err(BridgeError::Encoding(e)) }.boxed_local(),
                };

                let metadata = metadata
                    .downcast_ref::<BridgeMetadata<T::Context>>()
                    .expect("failed to downcast the context!")
                    .duplicate();

                async move {
                    let result = T::resolve(&metadata, &input).await;
                    bincode::serialize(&result.as_deref()).map_err(BridgeError::Encoding)
                }
                .boxed_local()
            });

            self.resolvers.push(resolver);
            self
        }

        pub(super) fn add_mutation_impl<T>(mut self) -> Self
        where
            T: 'static + BridgedMutation,
        {
            let ctx_id = TypeId::of::<T::Context>();
            if let Some(m) = self.ctx_id {
                assert_eq!(
                    m, ctx_id,
                    "all resolvers should have the same context type!"
                );
            }
            self.ctx_id = Some(ctx_id);

            let resolver = Arc::new(|metadata: &dyn Any, input: &[u8]| {
                let input = match bincode::deserialize::<T::Input>(input) {
                    Ok(m) => m,
                    Err(e) => return async move { Err(BridgeError::Encoding(e)) }.boxed_local(),
                };

                let metadata = metadata
                    .downcast_ref::<BridgeMetadata<T::Context>>()
                    .expect("failed to downcast the context!")
                    .duplicate();

                async move {
                    let result = T::resolve(&metadata, &input).await;
                    bincode::serialize(&result.as_deref()).map_err(BridgeError::Encoding)
                }
                .boxed_local()
            });

            self.resolvers.push(resolver);

            self
        }

        pub(super) fn with_token_selector_impl<T>(self) -> Self
        where
            T: 'static + Selector + AsRef<str>,
        {
            self
        }
    }
}
#[cfg(feature = "resolvable")]
use feat_resolvable::*;

#[cfg(not(feature = "resolvable"))]
mod not_feat_resolvable {
    use std::rc::Rc;

    use bounce::{BounceStates, Selector};
    use gloo_net::http::Request;
    use js_sys::Uint8Array;

    use super::*;
    pub(super) use crate::types::{BridgedMutation, BridgedQuery};

    pub(super) type ReadToken = Box<dyn Fn(&BounceStates) -> Rc<dyn AsRef<str>>>;

    impl Bridge {
        pub(crate) fn read_token(&self, states: &BounceStates) -> Option<Rc<dyn AsRef<str>>> {
            self.inner.read_token.as_ref().map(|m| m(states))
        }
    }

    impl<CTX> ConnectedBridge<CTX> {
        async fn resolve_encoded(&self, type_id: TypeId, input: &[u8]) -> BridgeResult<Vec<u8>> {
            let idx = self
                .inner
                .inner
                .query_ids
                .iter()
                .enumerate()
                .find(|(_, m)| **m == type_id)
                .ok_or(BridgeError::InvalidType(type_id))?
                .0;

            let incoming = Incoming {
                query_index: idx,
                input,
            };

            let incoming = bincode::serialize(&incoming)?;

            let input = Uint8Array::from(incoming.as_slice());
            let mut req = Request::post("/_bridge")
                .header("content-type", "application/x-bincode")
                .body(input);

            if let Some(m) = self.metadata.token() {
                req = req.header("authorization", &format!("Bearer {}", m));
            }

            let resp = req.send().await?;

            resp.binary().await.map_err(|m| m.into())
        }

        pub(crate) async fn resolve_query<T>(&self, input: &T::Input) -> QueryResult<T>
        where
            T: 'static + BridgedQuery,
        {
            let inner = move || async move {
                let input = bincode::serialize(&input).map_err(BridgeError::Encoding)?;
                let type_id = TypeId::of::<T>();

                let output = self.resolve_encoded(type_id, &input).await?;
                bincode::deserialize::<std::result::Result<T, T::Error>>(&output)
                    .map_err(BridgeError::Encoding)
            };

            inner().await.map_err(T::into_query_error)?.map(Rc::new)
        }

        pub(crate) async fn resolve_mutation<T>(&self, input: &T::Input) -> MutationResult<T>
        where
            T: 'static + BridgedMutation,
        {
            let inner = move || async move {
                let input = bincode::serialize(&input).map_err(BridgeError::Encoding)?;
                let type_id = TypeId::of::<T>();

                let output = self.resolve_encoded(type_id, &input).await?;
                bincode::deserialize::<std::result::Result<T, T::Error>>(&output)
                    .map_err(BridgeError::Encoding)
            };

            inner().await.map_err(T::into_mutation_error)?.map(Rc::new)
        }
    }

    impl BridgeBuilder {
        pub(super) fn add_query_impl<T>(mut self) -> Self
        where
            T: 'static + BridgedQuery,
        {
            let type_id = TypeId::of::<T>();
            self.query_ids.push(type_id);

            self
        }

        pub(super) fn add_mutation_impl<T>(mut self) -> Self
        where
            T: 'static + BridgedMutation,
        {
            let type_id = TypeId::of::<T>();
            self.query_ids.push(type_id);

            self
        }

        pub(super) fn with_token_selector_impl<T>(mut self) -> Self
        where
            T: 'static + Selector + AsRef<str>,
        {
            let read_token = Box::new(move |states: &BounceStates| {
                let state = states.get_selector_value::<T>();

                state as Rc<dyn AsRef<str>>
            }) as ReadToken;

            self.read_token = Some(read_token);

            self
        }
    }
}
#[cfg(not(feature = "resolvable"))]
use not_feat_resolvable::*;

/// A bridge to resolve requests.
///
/// See module documentation for more information.
pub struct Bridge {
    inner: Arc<BridgeBuilder>,
    id: usize,
}

impl fmt::Debug for Bridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Bridge")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl PartialEq for Bridge {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Bridge {}

impl Bridge {
    /// Creates a bridge builder.
    pub fn builder() -> BridgeBuilder {
        BridgeBuilder::default()
    }

    /// Connects a bridge.
    pub async fn connect<CTX>(
        self,
        metadata: BridgeMetadata<CTX>,
    ) -> BridgeResult<ConnectedBridge<CTX>>
    where
        CTX: 'static,
    {
        #[cfg(feature = "resolvable")]
        if let Some(m) = self.inner.ctx_id {
            assert_eq!(m, TypeId::of::<CTX>(), "context type do not match!");
        }

        Ok(ConnectedBridge {
            metadata,
            inner: self,
        })
    }
}

impl Default for Bridge {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl Clone for Bridge {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            id: self.id,
        }
    }
}

/// A connected bridge.
#[derive(Debug)]
pub struct ConnectedBridge<CTX> {
    metadata: BridgeMetadata<CTX>,
    inner: Bridge,
}
