//! States used by the frontend and backend.
//!
//! These states are registered automatically if you use backend endpoint or frontend renderer.

use std::rc::Rc;

use bounce::Atom;

use crate::{Bridge, BridgeMetadata};

/// The bridge state.
#[derive(Atom, PartialEq, Eq, Default, Debug)]
pub struct BridgeState {
    pub inner: Bridge,
}

/// The bridge metadata state.
#[derive(Atom, Debug)]
pub struct BridgeMetadataState<CTX> {
    pub(crate) _inner: Option<Rc<BridgeMetadata<CTX>>>,
}

impl<CTX> PartialEq for BridgeMetadataState<CTX> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<CTX> Default for BridgeMetadataState<CTX> {
    fn default() -> Self {
        Self { _inner: None }
    }
}

impl<CTX> From<Rc<BridgeMetadata<CTX>>> for BridgeMetadataState<CTX> {
    fn from(m: Rc<BridgeMetadata<CTX>>) -> Self {
        Self { _inner: Some(m) }
    }
}
