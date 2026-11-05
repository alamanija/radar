//! Bridge between GPUI's executors and the shared tokio runtime.
//!
//! GPUI's foreground/background executors are not tokio, but reqwest (and
//! everything in `radar-core` that does IO) needs a tokio reactor. Run such
//! futures through [`on_rt`] and await the result from any GPUI task:
//!
//! ```ignore
//! cx.spawn(async move |this, cx| {
//!     let resp = bridge::on_rt(ingest::run_briefing(inputs)).await;
//!     this.update(cx, |m, cx| m.update_model(cx, |r| r.finish_briefing(..)))
//! }).detach();
//! ```
//!
//! [`ReqwestHttpClient`] is the GPUI `HttpClient` (used by `img(url)`),
//! built on the same bridge.

use futures::future::BoxFuture;
use futures::AsyncReadExt as _;
use gpui_kit::http_client::http::HeaderValue;
use gpui_kit::http_client::{
    AsyncBody, HttpClient, RedirectPolicy, Request, RequestTimeout, Response, Url,
};
use std::future::Future;
use std::time::Duration;

/// Spawns `fut` on `radar_core::runtime()` right away and resolves to its
/// output. A panic inside `fut` is resumed in the awaiting task.
pub fn on_rt<T: Send + 'static>(
    fut: impl Future<Output = T> + Send + 'static,
) -> impl Future<Output = T> {
    let handle = radar_core::runtime().spawn(fut);
    async move {
        match handle.await {
            Ok(v) => v,
            Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Err(e) => panic!("tokio task cancelled: {e}"),
        }
    }
}

/// Runs blocking `f` (keychain I/O) on the runtime's blocking pool, never
/// the UI thread, and resolves to its output. A panic inside `f` is resumed
/// in the awaiting task.
pub fn on_blocking<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> impl Future<Output = T> {
    let handle = radar_core::runtime().spawn_blocking(f);
    async move {
        match handle.await {
            Ok(v) => v,
            Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Err(e) => panic!("blocking task cancelled: {e}"),
        }
    }
}

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_REDIRECTS: usize = 10;

/// GPUI's default client fails every request; this one runs reqwest on the
/// shared runtime. Bodies are buffered in memory (images, small JSON).
pub struct ReqwestHttpClient {
    follow: reqwest::Client,
    no_follow: reqwest::Client,
    user_agent: HeaderValue,
}

impl ReqwestHttpClient {
    pub fn new() -> Self {
        let ua = concat!("Radar/", env!("CARGO_PKG_VERSION"));
        let build = |policy| {
            reqwest::Client::builder()
                .user_agent(ua)
                .timeout(HTTP_TIMEOUT)
                .redirect(policy)
                .build()
                .expect("reqwest client")
        };
        Self {
            follow: build(reqwest::redirect::Policy::limited(MAX_REDIRECTS)),
            no_follow: build(reqwest::redirect::Policy::none()),
            user_agent: HeaderValue::from_static(ua),
        }
    }
}

impl Default for ReqwestHttpClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpClient for ReqwestHttpClient {
    fn user_agent(&self) -> Option<&HeaderValue> {
        Some(&self.user_agent)
    }

    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn send(
        &self,
        req: Request<AsyncBody>,
    ) -> BoxFuture<'static, anyhow::Result<Response<AsyncBody>>> {
        let (parts, mut body) = req.into_parts();
        // `FollowLimit(n)` is approximated by the fixed MAX_REDIRECTS limit.
        let client = match parts.extensions.get::<RedirectPolicy>() {
            None | Some(RedirectPolicy::NoFollow) => self.no_follow.clone(),
            Some(_) => self.follow.clone(),
        };
        let timeout = parts.extensions.get::<RequestTimeout>().map(|t| t.0);
        Box::pin(async move {
            let mut bytes = Vec::new();
            body.read_to_end(&mut bytes).await?;
            let mut rb = client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(bytes);
            if let Some(t) = timeout {
                rb = rb.timeout(t);
            }
            let (status, version, headers, body) = on_rt(async move {
                let resp = rb.send().await?;
                let (status, version, headers) =
                    (resp.status(), resp.version(), resp.headers().clone());
                let body = resp.bytes().await?;
                Ok::<_, reqwest::Error>((status, version, headers, body))
            })
            .await?;
            let mut out = Response::builder().status(status).version(version);
            if let Some(h) = out.headers_mut() {
                *h = headers;
            }
            Ok(out.body(AsyncBody::from(body.to_vec()))?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::http_client::AsyncBody;

    #[test]
    fn on_rt_runs_inside_tokio() {
        // tokio::time needs a tokio reactor; a plain executor has none.
        let v = futures::executor::block_on(on_rt(async {
            tokio::time::sleep(Duration::from_millis(1)).await;
            42
        }));
        assert_eq!(v, 42);
    }

    #[test]
    fn http_client_fetches_through_runtime() {
        let server = httpmock::MockServer::start();
        let m = server.mock(|when, then| {
            when.method("GET").path("/img.png");
            then.status(200).body("PNG");
        });
        let client = ReqwestHttpClient::new();
        let resp = futures::executor::block_on(client.get(
            &server.url("/img.png"),
            AsyncBody::empty(),
            true,
        ))
        .unwrap();
        assert_eq!(resp.status(), 200);
        let mut body = String::new();
        futures::executor::block_on(resp.into_body().read_to_string(&mut body)).unwrap();
        assert_eq!(body, "PNG");
        m.assert();
    }
}
