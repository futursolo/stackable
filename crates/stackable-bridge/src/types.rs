use std::error::Error;
use std::hash::Hash;
use std::marker::PhantomData;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::error::BridgeError;

#[cold]
fn panic_network_error(e: BridgeError) -> ! {
    panic!("failed to communicate with server: {:?}", e);
}

/// A Bridged Query.
///
/// This types defines a request that does not incur any side-effect on the server.
/// This type is cachable and will only resolve once until refreshed.
pub trait BridgedQuery: Serialize + for<'de> Deserialize<'de> + PartialEq {
    /// The Query Input.
    type Input: 'static + Serialize + for<'de> Deserialize<'de> + Hash + Eq + Clone;
    /// The Query Error Type.
    type Error: 'static + Serialize + for<'de> Deserialize<'de> + Error + PartialEq + Clone;

    /// Converts a BridgeError into the error type of current query.
    ///
    /// # Panics
    ///
    /// The default behaviour of a network error is panic.
    /// Override this method to make the error fallible.
    #[cold]
    fn into_query_error(e: BridgeError) -> Self::Error {
        panic_network_error(e);
    }
}

/// The query result type.
pub type QueryResult<T> = std::result::Result<Rc<T>, <T as BridgedQuery>::Error>;

/// A Bridged Mutation.
///
/// This types defines a request that incur side-effects on the server / cannot be cached.
pub trait BridgedMutation: Serialize + for<'de> Deserialize<'de> + PartialEq {
    /// The Mutation Input.
    type Input: 'static + Serialize + for<'de> Deserialize<'de>;
    /// The Mutation Error.
    type Error: 'static + Serialize + for<'de> Deserialize<'de> + Error + PartialEq + Clone;

    /// Converts a BridgeError into the error type of current mutation.
    ///
    /// # Panics
    ///
    /// The default behaviour of a network error is panic.
    /// Override this method to make the error fallible.
    #[cold]
    fn into_mutation_error(e: BridgeError) -> Self::Error {
        panic_network_error(e);
    }
}

/// The mutation result type.
pub type MutationResult<T> = std::result::Result<Rc<T>, <T as BridgedMutation>::Error>;

/// A placeholder type until never type lands in std.
#[derive(thiserror::Error, Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
#[error("this never happens")]
pub struct Never(PhantomData<()>);
