use std::sync::Arc;

/// The metadata of the bridge connection.
///
/// This metadata is provided when connecting the bridge.
#[derive(Debug, Default)]
pub struct BridgeMetadata<CTX = ()> {
    token: Option<Arc<str>>,
    context: Arc<CTX>,
}

impl<CTX> BridgeMetadata<CTX> {
    /// Creates a bridge metadata.
    pub fn new() -> Self
    where
        CTX: Default,
    {
        BridgeMetadata::default()
    }

    /// Sets the token used by the connection.
    pub fn with_token<S>(mut self, token: S) -> Self
    where
        S: AsRef<str>,
    {
        self.token = Some(token.as_ref().into());
        self
    }

    /// Sets the context used by the connection.
    ///
    /// This can be used to provide resolvers with additional information to help resolve the
    /// request.
    pub fn with_context<C>(self, context: C) -> BridgeMetadata<C> {
        BridgeMetadata {
            token: self.token,
            context: context.into(),
        }
    }

    /// Accesses the context.
    pub fn context(&self) -> &CTX {
        self.context.as_ref()
    }

    /// Gets the token, if it is available.
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Duplicates the context.
    ///
    /// The context intentionally does not implement clone. This is used to provide a clonable with
    /// crate visibility.
    #[cfg(feature = "resolvable")]
    pub(crate) fn duplicate(&self) -> Self {
        Self {
            token: self.token.clone(),
            context: self.context.clone(),
        }
    }
}
