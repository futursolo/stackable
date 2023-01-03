//! Stackable Frontend.
//!
//! This crate contains the frontend renderer and useful utilities for stackable applications.

#![deny(clippy::all)]
#![deny(missing_debug_implementations)]

use std::marker::PhantomData;

use stackable_bridge::Bridge;
use yew::prelude::*;

use crate::root::{StackableRoot, StackableRootProps};
pub mod components;
mod root;
pub mod trace;

/// The Stackable Frontend Renderer.
///
/// This type wraps the [Yew Renderer](yew::Renderer) and provides additional features.
#[derive(Debug)]
pub struct Renderer<COMP>
where
    COMP: BaseComponent,
{
    props: COMP::Properties,
    bridge: Option<Bridge>,
    _marker: PhantomData<COMP>,
}

impl<COMP> Default for Renderer<COMP>
where
    COMP: BaseComponent,
    COMP::Properties: Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<COMP> Renderer<COMP>
where
    COMP: BaseComponent,
{
    /// Creates a Renderer with default props.
    pub fn new() -> Renderer<COMP>
    where
        COMP::Properties: Default,
    {
        Self::with_props(Default::default())
    }

    /// Creates a Renderer with specified props.
    pub fn with_props(props: COMP::Properties) -> Renderer<COMP> {
        Renderer {
            props,
            bridge: None,
            _marker: PhantomData,
        }
    }

    /// Connects a bridge to the application.
    pub fn bridge(mut self, bridge: Bridge) -> Self {
        self.bridge = Some(bridge);

        self
    }

    fn into_yew_renderer(self) -> yew::Renderer<StackableRoot<COMP>> {
        let Self { props, bridge, .. } = self;
        let bridge = bridge.unwrap_or_default();

        let children = html! {
            <COMP ..props />
        };

        let props = StackableRootProps { bridge, children };

        yew::Renderer::with_props(props)
    }

    /// Renders the application.
    ///
    /// Whether the application is rendered or hydrated is determined automatically based on whether
    /// SSR is used on the server side for this page.
    pub fn render(self) {
        let renderer = self.into_yew_renderer();

        if web_sys::window()
            .and_then(|m| m.document())
            .and_then(|m| {
                m.query_selector(r#"meta[name="stackable-mode"][content="hydrate"]"#)
                    .ok()
                    .flatten()
            })
            .is_some()
        {
            renderer.hydrate();
        } else {
            renderer.render();
        }
    }
}
