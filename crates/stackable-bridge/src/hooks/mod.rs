//! Hooks used to resolve requests.

mod use_bridged_mutation;
mod use_bridged_query;

pub use use_bridged_mutation::{use_bridged_mutation, UseBridgedMutationHandle};
pub use use_bridged_query::{use_bridged_query, UseBridgedQueryHandle};
