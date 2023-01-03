use core::fmt;
use std::marker::PhantomData;

use futures::future::LocalBoxFuture;
use futures::{Future, FutureExt};
use stackable_bridge::{Bridge, BridgeMetadata};
use yew::prelude::*;

use crate::props::ServerAppProps;
use crate::utils::ThreadLocalLazy;

type BoxedSendFn<IN, OUT> = Box<dyn Send + Fn(IN) -> LocalBoxFuture<'static, OUT>>;
type SendFn<IN, OUT> = ThreadLocalLazy<BoxedSendFn<IN, OUT>>;

pub struct Endpoint<COMP, CTX = (), BCTX = ()>
where
    COMP: BaseComponent,
{
    #[allow(dead_code)]
    affix_context: SendFn<ServerAppProps<()>, ServerAppProps<CTX>>,
    bridge: Option<Bridge>,
    #[allow(dead_code)]
    affix_bridge_context: SendFn<BridgeMetadata<()>, BridgeMetadata<BCTX>>,
    #[cfg(feature = "warp-filter")]
    frontend: Option<crate::Frontend>,

    #[cfg(feature = "warp-filter")]
    auto_refresh: bool,

    _marker: PhantomData<COMP>,
}

impl<COMP, CTX, BCTX> fmt::Debug for Endpoint<COMP, CTX, BCTX>
where
    COMP: BaseComponent,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Endpoint<_>")
    }
}

impl<COMP, CTX, BCTX> Default for Endpoint<COMP, CTX, BCTX>
where
    COMP: BaseComponent,
    CTX: 'static + Default,
    BCTX: 'static + Default,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<COMP, CTX, BCTX> Endpoint<COMP, CTX, BCTX>
where
    COMP: BaseComponent,
    CTX: 'static,
    BCTX: 'static,
{
    pub fn new() -> Self
    where
        CTX: Default,
        BCTX: Default,
    {
        Self {
            affix_context: SendFn::<ServerAppProps<()>, ServerAppProps<CTX>>::new(move || {
                Box::new(|m| async move { m.with_context(CTX::default()) }.boxed())
            }),
            affix_bridge_context: SendFn::<BridgeMetadata<()>, BridgeMetadata<BCTX>>::new(
                move || Box::new(|m| async move { m.with_context(BCTX::default()) }.boxed()),
            ),
            bridge: None,
            #[cfg(feature = "warp-filter")]
            frontend: None,
            #[cfg(feature = "warp-filter")]
            auto_refresh: false,
            _marker: PhantomData,
        }
    }

    pub fn with_append_context<F, C, Fut>(self, append_context: F) -> Endpoint<COMP, C, BCTX>
    where
        F: 'static + Clone + Send + Fn(ServerAppProps<()>) -> Fut,
        Fut: 'static + Future<Output = ServerAppProps<C>>,
        C: 'static,
    {
        Endpoint {
            affix_context: SendFn::<ServerAppProps<()>, ServerAppProps<C>>::new(move || {
                let append_context = append_context.clone();
                Box::new(move |input| append_context(input).boxed_local())
            }),
            affix_bridge_context: self.affix_bridge_context,
            bridge: self.bridge,
            #[cfg(feature = "warp-filter")]
            frontend: self.frontend,
            #[cfg(feature = "warp-filter")]
            auto_refresh: self.auto_refresh,
            _marker: PhantomData,
        }
    }

    pub fn with_append_bridge_context<F, C, Fut>(
        self,
        append_bridge_context: F,
    ) -> Endpoint<COMP, CTX, C>
    where
        F: 'static + Clone + Send + Fn(BridgeMetadata<()>) -> Fut,
        Fut: 'static + Future<Output = BridgeMetadata<C>>,
        C: 'static,
    {
        Endpoint {
            affix_context: self.affix_context,
            affix_bridge_context: SendFn::<BridgeMetadata<()>, BridgeMetadata<C>>::new(move || {
                let append_bridge_context = append_bridge_context.clone();
                Box::new(move |input| append_bridge_context(input).boxed_local())
            }),
            bridge: self.bridge,
            #[cfg(feature = "warp-filter")]
            frontend: self.frontend,
            #[cfg(feature = "warp-filter")]
            auto_refresh: self.auto_refresh,
            _marker: PhantomData,
        }
    }

    pub fn with_bridge(mut self, bridge: Bridge) -> Self {
        self.bridge = Some(bridge);
        self
    }
}

#[cfg(feature = "warp-filter")]
mod feat_warp_filter {
    use std::fmt::Write;
    use std::future::Future;
    use std::ops::Deref;
    use std::rc::Rc;

    use bounce::helmet::render_static;
    use bytes::Bytes;
    use futures::{SinkExt, StreamExt, TryFutureExt};
    use http::status::StatusCode;
    use once_cell::sync::Lazy;
    use stackable_bridge::{BridgeError, BridgeMetadata};
    use tokio::sync::oneshot as sync_oneshot;
    use warp::body::bytes;
    use warp::path::FullPath;
    use warp::reject::not_found;
    use warp::reply::Response;
    use warp::ws::{Message, Ws};
    use warp::{header, log, reply, Filter, Rejection, Reply};
    use yew::platform::{LocalHandle, Runtime};

    use super::*;
    use crate::root::{StackableRoot, StackableRootProps};
    use crate::utils::random_str;
    use crate::Frontend;

    // A server id that is different every time it starts.
    static SERVER_ID: Lazy<String> = Lazy::new(random_str);

    static AUTO_REFRESH_SCRIPT: Lazy<String> = Lazy::new(|| {
        format!(
            r#"
<script>
    (() => {{
        const protocol = window.location.protocol === 'https' ? 'wss' : 'ws';
        const wsUrl = `${{protocol}}://${{window.location.host}}/_refresh`;
        const serverId = '{}';

        const connectWs = () => {{
            const ws = new WebSocket(wsUrl);
            ws.addEventListener('open', () => {{
                const invId = setInterval(() => {{
                    try {{
                        ws.send(serverId);
                    }} catch(e) {{
                        // do nothing if errored.
                    }}
                }}, 1000);
                ws.addEventListener('error', () => {{
                    clearInterval(invId);
                }});
            }});
            ws.addEventListener('close', () => {{
                setTimeout(connectWs, 1000);
            }});
            ws.addEventListener('message', (e) => {{
                if (e.data === 'restart') {{
                    window.location.reload();
                }}
            }});
        }};

        connectWs();
    }})();
</script>"#,
            SERVER_ID.as_str()
        )
    });

    impl<COMP, CTX, BCTX> Endpoint<COMP, CTX, BCTX>
    where
        COMP: BaseComponent<Properties = ServerAppProps<CTX>>,
        CTX: 'static,
        BCTX: 'static,
    {
        pub fn with_auto_refresh(mut self) -> Self {
            self.auto_refresh = true;

            self
        }

        fn create_index_filter(
            &self,
        ) -> Option<
            impl Clone
                + Send
                + Filter<
                    Extract = (Response,),
                    Error = Rejection,
                    Future = impl Future<Output = Result<(Response,), Rejection>>,
                >,
        > {
            let index_html = self.frontend.as_ref()?.index_html();
            let affix_context = self.affix_context.clone();
            let bridge = self.bridge.clone().unwrap_or_default();
            let auto_refresh = self.auto_refresh;
            let affix_bridge_context = self.affix_bridge_context.clone();

            let create_render_inner = move |props, tx: sync_oneshot::Sender<String>| async move {
                let props = (affix_context.deref())(props).await;
                let bridge_metadata =
                    Rc::new((affix_bridge_context.deref())(BridgeMetadata::new()).await);

                let mut head_s = String::new();
                let mut body_s = String::new();
                let mut helmet_tags = Vec::new();

                if !props.is_client_only() {
                    let (reader, writer) = render_static();

                    body_s =
                        yew::LocalServerRenderer::<StackableRoot<COMP, CTX, BCTX>>::with_props(
                            StackableRootProps {
                                server_app_props: props,
                                helmet_writer: writer,
                                bridge,
                                bridge_metadata,
                            },
                        )
                        .render()
                        .await;

                    helmet_tags = reader.render().await;
                    let _ = write!(
                        &mut head_s,
                        r#"<meta name="stackable-mode" content="hydrate">"#
                    );
                }

                // With development server, we read index.html every time.
                if auto_refresh {
                    body_s.push_str(AUTO_REFRESH_SCRIPT.as_str());
                }

                let s = index_html.render(helmet_tags, head_s, body_s).await;
                let _ = tx.send(s);
            };

            let render_html = move |props| async move {
                let (tx, rx) = sync_oneshot::channel::<String>();

                // We spawn into a local runtime early for higher efficiency.
                match LocalHandle::try_current() {
                    Some(handle) => handle.spawn_local(create_render_inner(props, tx)),
                    // TODO: Allow Overriding Runtime with Endpoint.
                    None => Runtime::default().spawn_pinned(move || create_render_inner(props, tx)),
                }

                warp::reply::html(rx.await.expect("renderer panicked?"))
            };

            let f = warp::get()
                .and(warp::path::full())
                .and(
                    warp::query::raw()
                        .or_else(|_| async move { Ok::<_, Rejection>((String::new(),)) }),
                )
                .then(move |path: FullPath, raw_queries| {
                    let props = ServerAppProps::from_warp_request(path, raw_queries);
                    let render_html = render_html.clone();

                    async move { render_html(props).await.into_response() }
                });

            Some(f)
        }

        fn create_refresh_filter(
        ) -> impl Clone + Send + Filter<Extract = (Response,), Error = Rejection> {
            warp::path::path("_refresh")
                .and(warp::ws())
                .then(|m: Ws| async move {
                    m.on_upgrade(|mut ws| async move {
                        let read_refresh = {
                            || async move {
                                while let Some(m) = ws.next().await {
                                    let m = match m {
                                        Ok(m) => m,
                                        Err(e) => {
                                            tracing::error!("receive message error: {:?}", e);

                                            if let Err(e) = ws.close().await {
                                                tracing::error!(
                                                    "failed to close websocket: {:?}",
                                                    e
                                                );
                                            }

                                            return;
                                        }
                                    };

                                    if m.is_ping() || m.is_pong() {
                                        continue;
                                    }

                                    let m = match m.to_str() {
                                        Ok(m) => m,
                                        Err(_) => {
                                            tracing::error!("received unknown message: {:?}", m);
                                            return;
                                        }
                                    };

                                    // Ping client if string matches.
                                    // Otherwise, tell the client to reload the page.
                                    let message_to_send = if m == SERVER_ID.as_str() {
                                        Message::ping("")
                                    } else {
                                        Message::text("restart")
                                    };

                                    if let Err(e) = ws.send(message_to_send).await {
                                        tracing::error!("error sending message: {:?}", e);
                                        return;
                                    }
                                }
                            }
                        };

                        match LocalHandle::try_current() {
                            Some(handle) => handle.spawn_local(read_refresh()),
                            // TODO: Allow Overriding Runtime with Endpoint.
                            None => Runtime::default().spawn_pinned(read_refresh),
                        }
                    })
                    .into_response()
                })
        }

        fn create_bridge_filter(
            &self,
        ) -> Option<impl Clone + Send + Filter<Extract = (Response,), Error = Rejection>> {
            let bridge = self.bridge.clone()?;

            let http_bridge_f = warp::post()
                .and(header::exact_ignore_case(
                    "content-type",
                    "application/x-bincode",
                ))
                .and(header::optional("authorization"))
                .and(bytes())
                .then(move |token: Option<String>, input: Bytes| {
                    let bridge = bridge.clone();
                    let (tx, rx) = sync_oneshot::channel();

                    let resolve_encoded = move || async move {
                        let mut meta = BridgeMetadata::<()>::new();

                        if let Some(m) = token {
                            if !m.starts_with("Bearer ") {
                                let reply =
                                    reply::with_status("", StatusCode::BAD_REQUEST).into_response();

                                let _ = tx.send(reply);
                                return;
                            }

                            meta = meta.with_token(m.split_at(7).1);
                        }

                        let content = bridge
                            .connect(meta)
                            .and_then(|m| async move { m.resolve_encoded(&input).await })
                            .await;

                        let reply = match content {
                            Ok(m) => reply::with_header(m, "content-type", "application/x-bincode")
                                .into_response(),
                            Err(BridgeError::Encoding(_))
                            | Err(BridgeError::InvalidIndex(_))
                            | Err(BridgeError::InvalidType(_)) => {
                                reply::with_status("", StatusCode::BAD_REQUEST).into_response()
                            }
                            Err(BridgeError::Network(_)) => {
                                reply::with_status("", StatusCode::INTERNAL_SERVER_ERROR)
                                    .into_response()
                            }
                        };

                        let _ = tx.send(reply);
                    };

                    match LocalHandle::try_current() {
                        Some(handle) => handle.spawn_local(resolve_encoded()),
                        // TODO: Allow Overriding Runtime with Endpoint.
                        None => Runtime::default().spawn_pinned(resolve_encoded),
                    }

                    async move { rx.await.expect("failed to resolve the bridge request") }
                });

            Some(warp::path::path("_bridge").and(http_bridge_f))
        }

        pub fn with_frontend(mut self, frontend: Frontend) -> Self {
            self.frontend = Some(frontend);

            self
        }

        pub fn into_warp_filter(
            self,
        ) -> impl Clone + Send + Filter<Extract = (impl Reply + Send,), Error = Rejection> {
            let bridge_f = self.create_bridge_filter();
            let index_html_f = self.create_index_filter();

            let Self { frontend, .. } = self;

            let mut routes = match index_html_f.clone() {
                None => warp::path::end()
                    .and_then(|| async move { Err::<Response, Rejection>(not_found()) })
                    .boxed(),
                Some(m) => warp::path::end().and(m).boxed(),
            };

            if let Some(m) = bridge_f {
                routes = routes.or(m).unify().boxed();
            }

            if let Some(m) = frontend {
                routes = routes.or(m.into_warp_filter()).unify().boxed();
            }

            if self.auto_refresh {
                routes = routes.or(Self::create_refresh_filter()).unify().boxed();
            }

            if let Some(m) = index_html_f {
                routes = routes.or(m).unify().boxed();
            }

            routes.with(log::custom(|info| {
                // We emit a custom span so it won't interfere with warp's default tracing event.
                tracing::info!(target: "stackable_backend::endpoint::trace",
                remote_addr = ?info.remote_addr(),
                method = %info.method(),
                path = info.path(),
                status = info.status().as_u16(),
                referer = ?info.referer(),
                user_agent = ?info.user_agent(),
                duration = info.elapsed().as_nanos());
            }))
        }
    }
}

#[cfg(feature = "tower-service")]
mod feat_tower_service {
    use std::convert::Infallible;
    use std::future::Future;

    use hyper::{Body, Request, Response};
    use tower::Service;

    use super::*;
    impl<COMP, CTX, BCTX> Endpoint<COMP, CTX, BCTX>
    where
        COMP: BaseComponent<Properties = ServerAppProps<CTX>>,
        CTX: 'static,
        BCTX: 'static,
    {
        pub fn into_tower_service(
            self,
        ) -> impl 'static
               + Clone
               + Service<
            Request<Body>,
            Response = Response<Body>,
            Error = Infallible,
            Future = impl 'static + Send + Future<Output = Result<Response<Body>, Infallible>>,
        > {
            let routes = self.into_warp_filter();
            warp::service(routes)
        }
    }
}
