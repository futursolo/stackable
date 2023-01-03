use std::borrow::Cow;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fmt, str};

use bounce::helmet::HelmetTag;
use lol_html::{doc_comments, element, rewrite_str, Settings};
use rust_embed::{EmbeddedFile, RustEmbed};
use tokio::fs;
use warp::filters::fs::File;
use warp::filters::BoxedFilter;
use warp::path::Tail;
use warp::reply::{with_header, Response};
use warp::{Filter, Rejection, Reply};

use crate::utils::ThreadLocalLazy;

type GetFileFn = Box<dyn Send + Fn(&str) -> Option<EmbeddedFile>>;

type GetFile = ThreadLocalLazy<GetFileFn>;

#[derive(Clone)]
enum Inner {
    Path(PathBuf),
    Embed { get_file: GetFile },
}

impl fmt::Debug for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Inner::Path(ref p) => f.debug_struct("Inner::Path").field("0", p).finish(),
            Inner::Embed { .. } => f.debug_struct("Inner::Embed").finish_non_exhaustive(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Frontend {
    inner: Inner,
}

impl Frontend {
    pub fn new_path<P>(p: P) -> Self
    where
        P: Into<PathBuf>,
    {
        let p = p.into();

        Self {
            inner: Inner::Path(p),
        }
    }

    pub fn new_embedded<E>() -> Self
    where
        E: RustEmbed,
    {
        let get_file = ThreadLocalLazy::new(|| Box::new(|path: &str| E::get(path)) as GetFileFn);

        Self {
            inner: Inner::Embed { get_file },
        }
    }

    pub(crate) fn into_warp_filter(self) -> BoxedFilter<(Response,)> {
        match self.inner {
            Inner::Path(m) => warp::fs::dir(m)
                .then(|m: File| async move { m.into_response() })
                .boxed(),
            Inner::Embed { get_file } => warp::path::tail()
                .and_then(move |path: Tail| {
                    let get_file = get_file.clone();
                    async move {
                        let get_file = get_file.deref();

                        let asset = get_file(path.as_str()).ok_or_else(warp::reject::not_found)?;
                        let mime = mime_guess::from_path(path.as_str()).first_or_octet_stream();

                        Ok::<_, Rejection>(
                            with_header(
                                warp::hyper::Response::new(asset.data),
                                "content-type",
                                mime.as_ref(),
                            )
                            .into_response(),
                        )
                    }
                })
                .boxed(),
        }
    }

    pub(crate) fn index_html(&self) -> IndexHtml {
        match self.inner {
            Inner::Path(ref m) => IndexHtml::Path(m.join("index.html").into()),
            Inner::Embed { ref get_file } => (get_file.deref())("index.html")
                .map(|m| m.data)
                .as_deref()
                .map(String::from_utf8_lossy)
                .map(Arc::from)
                .map(IndexHtml::Embedded)
                .expect("index.html not found!"),
        }
    }
}

#[derive(Clone)]
pub(crate) enum IndexHtml {
    Embedded(Arc<str>),
    Path(Arc<Path>),
}

impl IndexHtml {
    async fn read_content(&self) -> Cow<'_, str> {
        match self {
            IndexHtml::Path(p) => fs::read_to_string(&p)
                .await
                .map(Cow::from)
                .expect("TODO: implement failure."),

            IndexHtml::Embedded(ref s) => s.as_ref().into(),
        }
    }

    pub async fn render<I, H, B>(&self, tags: I, head_s: H, body_s: B) -> String
    where
        I: IntoIterator<Item = HelmetTag>,
        H: Into<String>,
        B: AsRef<str>,
    {
        let mut head_s = head_s.into();
        let body_s = body_s.as_ref();

        let mut html_tag = None;
        let mut body_tag = None;

        for tag in tags.into_iter() {
            match tag {
                HelmetTag::Html { .. } => {
                    html_tag = Some(tag);
                }
                HelmetTag::Body { .. } => {
                    body_tag = Some(tag);
                }
                _ => {
                    let _ = tag.write_static(&mut head_s);
                }
            }
        }

        let index_html_s = self.read_content().await;

        rewrite_str(
            &index_html_s,
            Settings {
                element_content_handlers: vec![
                    element!("html", |h| {
                        if let Some(HelmetTag::Html { attrs }) = html_tag.take() {
                            for (k, v) in attrs {
                                h.set_attribute(k.as_ref(), v.as_ref())?;
                            }
                        }

                        Ok(())
                    }),
                    element!("body", |h| {
                        if let Some(HelmetTag::Body { attrs }) = body_tag.take() {
                            for (k, v) in attrs {
                                h.set_attribute(k.as_ref(), v.as_ref())?;
                            }
                        }

                        Ok(())
                    }),
                ],

                document_content_handlers: vec![doc_comments!(|c| {
                    if c.text() == "%STACKABLE_HEAD%" {
                        c.replace(&head_s, lol_html::html_content::ContentType::Html);
                    }
                    if c.text() == "%STACKABLE_BODY%" {
                        c.replace(body_s, lol_html::html_content::ContentType::Html);
                    }

                    Ok(())
                })],
                ..Default::default()
            },
        )
        .expect("failed to render html")
    }
}
