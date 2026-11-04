//! Clerk OAuth 2.0 sign-in for a public desktop client (no client secret):
//! Authorization Code + PKCE (S256) through the system browser with a
//! loopback redirect on `127.0.0.1`, token refresh, and a keychain-backed
//! [`Session`].
//!
//! Security notes:
//! - Access/refresh/id tokens, the authorization code and the PKCE verifier
//!   are never logged and never appear in [`AuthError`] messages.
//! - The id_token is decoded WITHOUT signature verification and used only to
//!   display who is signed in. Authorization happens server-side, where
//!   radar-server verifies the access token.

use crate::keychain::{KeyStore, CLERK_OAUTH_USER};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use futures::stream::{FuturesUnordered, StreamExt};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::net::Ipv4Addr;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub use crate::types::Account;

/// Loopback port registered as `http://127.0.0.1:17823/callback` in Clerk.
pub const DEFAULT_REDIRECT_PORT: u16 = 17823;
const SCOPE: &str = "openid profile email offline_access";
/// Refresh when the access token has fewer than this many seconds left.
const REFRESH_MARGIN_SECS: i64 = 60;
/// Used when the token response omits `expires_in`.
const DEFAULT_EXPIRES_IN_SECS: i64 = 3600;
const MAX_REQUEST_HEAD: usize = 16 * 1024;
/// Per-connection read budget, so an idle (pre)connection can't stall the flow.
const CONN_READ_TIMEOUT: Duration = Duration::from_secs(10);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
const ERROR_SNIPPET_CHARS: usize = 200;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .expect("failed to build auth HTTP client")
});

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthConfig {
    /// Clerk instance Frontend API origin, e.g. `https://clerk.example.com`
    /// (no trailing slash).
    pub frontend_api: String,
    /// OAuth application client id (public client).
    pub client_id: String,
    /// Loopback port for the redirect; `0` binds an ephemeral port (tests).
    pub redirect_port: u16,
}

impl AuthConfig {
    /// Reads `RADAR_CLERK_FRONTEND_API` and `RADAR_CLERK_CLIENT_ID` from the
    /// runtime environment, falling back to values baked in at compile time.
    /// `None` when either is missing or empty.
    pub fn from_env() -> Option<Self> {
        Self::from_values(
            env_value(
                std::env::var("RADAR_CLERK_FRONTEND_API").ok(),
                option_env!("RADAR_CLERK_FRONTEND_API"),
            ),
            env_value(
                std::env::var("RADAR_CLERK_CLIENT_ID").ok(),
                option_env!("RADAR_CLERK_CLIENT_ID"),
            ),
        )
    }

    fn from_values(frontend_api: Option<String>, client_id: Option<String>) -> Option<Self> {
        let frontend_api = frontend_api?.trim().trim_end_matches('/').to_string();
        let client_id = client_id?.trim().to_string();
        if frontend_api.is_empty() || client_id.is_empty() {
            return None;
        }
        Some(Self {
            frontend_api,
            client_id,
            redirect_port: DEFAULT_REDIRECT_PORT,
        })
    }

    /// The browser URL that starts sign-in, redirecting back to `port`.
    pub fn authorize_url(&self, pkce: &Pkce, port: u16) -> String {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &redirect_uri(port))
            .append_pair("scope", SCOPE)
            .append_pair("state", &pkce.state)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .finish();
        format!("{}/oauth/authorize?{query}", self.frontend_api)
    }

    fn token_url(&self) -> String {
        format!("{}/oauth/token", self.frontend_api)
    }
}

/// Runtime value if set and non-empty, else the compile-time one.
fn env_value(runtime: Option<String>, compiled: Option<&str>) -> Option<String> {
    runtime.filter(|v| !v.trim().is_empty()).or_else(|| {
        compiled
            .filter(|v| !v.trim().is_empty())
            .map(str::to_string)
    })
}

fn redirect_uri(port: u16) -> String {
    format!("http://127.0.0.1:{port}/callback")
}

// ---------------------------------------------------------------------------
// Tokens, PKCE, errors
// ---------------------------------------------------------------------------

/// OAuth tokens as persisted in the keychain (JSON under `clerk_oauth`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
    pub id_token: Option<String>,
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const R: &str = "<redacted>";
        f.debug_struct("Tokens")
            .field("access_token", &R)
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| R))
            .field("expires_at", &self.expires_at)
            .field("id_token", &self.id_token.as_ref().map(|_| R))
            .finish()
    }
}

/// PKCE (RFC 7636, S256) verifier/challenge plus the CSRF `state`.
pub struct Pkce {
    /// 64 URL-safe characters (48 random bytes).
    pub verifier: String,
    /// `BASE64URL(SHA256(verifier))`.
    pub challenge: String,
    pub state: String,
}

impl Pkce {
    pub fn new() -> Self {
        let verifier = random_urlsafe(48);
        Self {
            challenge: challenge_for(&verifier),
            verifier,
            state: random_urlsafe(24),
        }
    }
}

impl std::fmt::Debug for Pkce {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pkce")
            .field("challenge", &self.challenge)
            .finish_non_exhaustive()
    }
}

impl Default for Pkce {
    fn default() -> Self {
        Self::new()
    }
}

fn random_urlsafe(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Sign-in / token errors. No variant ever carries a token, code or verifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// The loopback redirect port couldn't be bound.
    PortInUse,
    /// No callback arrived before the deadline.
    Timeout,
    /// The provider returned `?error=<code>` to the callback.
    Denied(String),
    /// The callback's `state` didn't match (possible CSRF).
    StateMismatch,
    /// Transport or protocol failure: status + a short non-secret snippet.
    Http(String),
    /// Clerk settings are missing.
    NotConfigured,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PortInUse => write!(
                f,
                "the sign-in callback port is in use; close other Radar windows and retry"
            ),
            Self::Timeout => write!(f, "timed out waiting for the browser sign-in"),
            Self::Denied(e) => write!(f, "sign-in was denied ({e})"),
            Self::StateMismatch => write!(f, "sign-in callback did not match this attempt"),
            Self::Http(m) => write!(f, "sign-in request failed: {m}"),
            Self::NotConfigured => write!(f, "Clerk sign-in is not configured"),
        }
    }
}

impl std::error::Error for AuthError {}

// ---------------------------------------------------------------------------
// JWT claims (unverified, display only)
// ---------------------------------------------------------------------------

/// Decodes a JWT's header and payload WITHOUT verifying the signature.
/// For display/debugging only — never base an authorization decision on it.
pub fn decode_jwt_unverified(jwt: &str) -> Option<(serde_json::Value, serde_json::Value)> {
    let mut parts = jwt.split('.');
    let (header, payload) = (parts.next()?, parts.next()?);
    parts.next()?; // a signature segment must be present
    let decode = |seg: &str| {
        let bytes = URL_SAFE_NO_PAD.decode(seg.trim_end_matches('=')).ok()?;
        serde_json::from_slice::<serde_json::Value>(&bytes).ok()
    };
    Some((decode(header)?, decode(payload)?))
}

/// Builds the display [`Account`] from an id_token's `sub/email/name/picture`.
pub fn account_from_id_token(jwt: &str) -> Option<Account> {
    let (_, claims) = decode_jwt_unverified(jwt)?;
    let claim = |k: &str| {
        claims
            .get(k)
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let name = claim("name").or_else(|| {
        let full = [claim("given_name"), claim("family_name")]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
        (!full.is_empty()).then_some(full)
    });
    Some(Account {
        id: claim("sub")?,
        email: claim("email").unwrap_or_default(),
        name: name.unwrap_or_default(),
        image_url: claim("picture"),
    })
}

/// Identity from the id_token, falling back to a JWT access token's claims.
fn account_of(tokens: &Tokens) -> Option<Account> {
    tokens
        .id_token
        .as_deref()
        .and_then(account_from_id_token)
        .or_else(|| account_from_id_token(&tokens.access_token))
}

// ---------------------------------------------------------------------------
// Token endpoint
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
}

impl TokenResponse {
    fn into_tokens(self, previous_refresh: Option<&str>) -> Tokens {
        let ttl = self.expires_in.unwrap_or(DEFAULT_EXPIRES_IN_SECS).max(0);
        Tokens {
            access_token: self.access_token,
            refresh_token: self
                .refresh_token
                .or_else(|| previous_refresh.map(str::to_string)),
            expires_at: now_unix() + ttl,
            id_token: self.id_token,
        }
    }
}

#[derive(Deserialize)]
struct OAuthErrorBody {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// A token-endpoint failure, keeping the RFC 6749 `error` code when sent.
struct TokenFailure {
    oauth_error: Option<String>,
    err: AuthError,
}

impl From<AuthError> for TokenFailure {
    fn from(err: AuthError) -> Self {
        Self {
            oauth_error: None,
            err,
        }
    }
}

/// POSTs a form to the token endpoint. Secret form values are scrubbed from
/// any error text in case the server echoes them back.
async fn post_token(
    cfg: &AuthConfig,
    form: &[(&str, &str)],
) -> Result<TokenResponse, TokenFailure> {
    let secrets: Vec<&str> = form
        .iter()
        .filter(|(k, _)| matches!(*k, "code" | "code_verifier" | "refresh_token"))
        .map(|(_, v)| *v)
        .filter(|v| !v.is_empty())
        .collect();
    let resp = HTTP
        .post(cfg.token_url())
        .header(reqwest::header::ACCEPT, "application/json")
        .form(form)
        .send()
        .await
        .map_err(|e| AuthError::Http(format!("token request: {e}")))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .await
        .map_err(|e| AuthError::Http(format!("token response HTTP {status}: {e}")))?;
    if !(200..300).contains(&status) {
        let oauth = serde_json::from_str::<OAuthErrorBody>(&body).ok();
        let detail = match &oauth {
            Some(o) => match &o.error_description {
                Some(d) => format!("{}: {d}", o.error),
                None => o.error.clone(),
            },
            None => body,
        };
        return Err(TokenFailure {
            oauth_error: oauth.map(|o| o.error),
            err: AuthError::Http(format!(
                "token endpoint HTTP {status}: {}",
                snippet(&detail, &secrets)
            )),
        });
    }
    // serde_json messages can quote offending values, so report only where.
    serde_json::from_str::<TokenResponse>(&body).map_err(|e| {
        AuthError::Http(format!(
            "token endpoint HTTP {status}: malformed response ({:?} at line {} column {})",
            e.classify(),
            e.line(),
            e.column()
        ))
        .into()
    })
}

/// A short, single-line, secret-scrubbed excerpt for error messages.
fn snippet(text: &str, secrets: &[&str]) -> String {
    let mut s = text.to_string();
    for secret in secrets {
        s = s.replace(secret, "<redacted>");
    }
    let mut out: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(ERROR_SNIPPET_CHARS)
        .collect();
    if s.chars().count() > ERROR_SNIPPET_CHARS {
        out.push('…');
    }
    out
}

/// Exchanges a refresh token for fresh tokens. A response without a new
/// refresh token keeps the one passed in.
pub async fn refresh(cfg: &AuthConfig, refresh_token: &str) -> Result<Tokens, AuthError> {
    refresh_inner(cfg, refresh_token).await.map_err(|f| f.err)
}

async fn refresh_inner(cfg: &AuthConfig, refresh_token: &str) -> Result<Tokens, TokenFailure> {
    let resp = post_token(
        cfg,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &cfg.client_id),
        ],
    )
    .await?;
    Ok(resp.into_tokens(Some(refresh_token)))
}

// ---------------------------------------------------------------------------
// Loopback sign-in
// ---------------------------------------------------------------------------

/// Runs the browser sign-in: binds `127.0.0.1:{cfg.redirect_port}`, calls
/// `open(authorize_url)`, waits up to `timeout` for `GET /callback`, then
/// exchanges the code (with the PKCE verifier) at `{frontend_api}/oauth/token`.
pub async fn sign_in(
    cfg: &AuthConfig,
    open: impl FnOnce(&str),
    timeout: Duration,
) -> Result<(Tokens, Account), AuthError> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, cfg.redirect_port))
        .await
        .map_err(|e| match e.kind() {
            // Windows reports reserved/excluded ports as access denied.
            std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied => {
                AuthError::PortInUse
            }
            _ => AuthError::Http(format!("loopback bind: {e}")),
        })?;
    let port = listener
        .local_addr()
        .map_err(|e| AuthError::Http(format!("loopback address: {e}")))?
        .port();
    let pkce = Pkce::new();
    log::info!("[auth] waiting for sign-in callback on 127.0.0.1:{port}");
    open(&cfg.authorize_url(&pkce, port));

    let code = tokio::time::timeout(timeout, wait_for_callback(listener, &pkce.state))
        .await
        .map_err(|_| AuthError::Timeout)??;

    let redirect = redirect_uri(port);
    let resp = post_token(
        cfg,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &redirect),
            ("client_id", &cfg.client_id),
            ("code_verifier", &pkce.verifier),
        ],
    )
    .await
    .map_err(|f| f.err)?;
    let tokens = resp.into_tokens(None);
    let account = account_of(&tokens)
        .ok_or_else(|| AuthError::Http("token response carried no identity claims".into()))?;
    log::info!("[auth] sign-in complete");
    Ok((tokens, account))
}

/// Serves connections until one hits `/callback`; returns its code. The
/// listener closes when this returns (or is dropped on timeout).
async fn wait_for_callback(listener: TcpListener, state: &str) -> Result<String, AuthError> {
    let mut conns = FuturesUnordered::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => conns.push(handle_connection(stream, state)),
                Err(e) => {
                    log::warn!("[auth] loopback accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            },
            Some(outcome) = conns.next(), if !conns.is_empty() => {
                if let Some(result) = outcome {
                    return result;
                }
            }
        }
    }
}

/// Handles one loopback connection. `None` means "not the callback, keep
/// waiting" (favicon, junk, idle preconnect).
async fn handle_connection(
    mut stream: TcpStream,
    state: &str,
) -> Option<Result<String, AuthError>> {
    let head = tokio::time::timeout(CONN_READ_TIMEOUT, read_request_head(&mut stream))
        .await
        .ok()??;
    let mut parts = head.lines().next()?.split_whitespace();
    let (method, target) = (parts.next()?, parts.next()?);
    if method != "GET" || !target.starts_with('/') {
        respond(&mut stream, 405, "Method Not Allowed", "").await;
        return None;
    }
    let Ok(url) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
        respond(&mut stream, 400, "Bad Request", "").await;
        return None;
    };
    if url.path() != "/callback" {
        respond(&mut stream, 404, "Not Found", "").await;
        return None;
    }
    let result = evaluate_callback(&url, state);
    match &result {
        Ok(_) => {
            let body = page(
                "Signed in to Radar",
                "You can close this tab and return to Radar.",
            );
            respond(&mut stream, 200, "OK", &body).await;
        }
        Err(e) => {
            let msg = match e {
                AuthError::Denied(_) => "Sign-in was cancelled. You can close this tab.",
                _ => "Sign-in failed. Close this tab and try again from Radar.",
            };
            respond(&mut stream, 400, "Bad Request", &page("Radar sign-in", msg)).await;
        }
    }
    Some(result)
}

fn evaluate_callback(url: &url::Url, expected_state: &str) -> Result<String, AuthError> {
    let (mut code, mut state, mut error) = (None, None, None);
    for (k, v) in url.query_pairs() {
        let slot = match k.as_ref() {
            "code" => &mut code,
            "state" => &mut state,
            "error" => &mut error,
            _ => continue,
        };
        slot.get_or_insert(v.into_owned());
    }
    if let Some(e) = error {
        let e: String = e.chars().filter(|c| !c.is_control()).take(64).collect();
        return Err(AuthError::Denied(e));
    }
    if state.as_deref() != Some(expected_state) {
        return Err(AuthError::StateMismatch);
    }
    code.filter(|c| !c.is_empty())
        .ok_or_else(|| AuthError::Http("callback carried no authorization code".into()))
}

/// Reads up to the end of the request head (GET has no body). Reading it all
/// avoids a TCP reset on close, which would hide our page from the browser.
async fn read_request_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_REQUEST_HEAD {
            return None;
        }
    }
    (!buf.is_empty()).then(|| String::from_utf8_lossy(&buf).into_owned())
}

async fn respond(stream: &mut TcpStream, status: u16, reason: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn page(title: &str, message: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head>\
         <body style=\"font-family:system-ui,sans-serif;text-align:center;padding:4rem\">\
         <h1>{title}</h1><p>{message}</p></body></html>"
    )
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// The signed-in session: tokens cached in memory and persisted in the
/// keychain. Cheap to clone; clones share state.
#[derive(Clone)]
pub struct Session {
    cfg: AuthConfig,
    store: Arc<dyn KeyStore>,
    tokens: Arc<Mutex<Option<Tokens>>>,
    /// Serializes refreshes so concurrent callers don't both spend the
    /// refresh token.
    refresh_lock: Arc<tokio::sync::Mutex<()>>,
}

impl Session {
    /// Loads tokens from `store`. Unreadable entries mean "signed out".
    pub fn load(cfg: AuthConfig, store: Arc<dyn KeyStore>) -> Self {
        let tokens = match store.get(CLERK_OAUTH_USER) {
            Ok(Some(json)) => serde_json::from_str::<Tokens>(&json)
                .map_err(|_| log::warn!("[auth] stored session is unreadable; signed out"))
                .ok(),
            Ok(None) => None,
            Err(e) => {
                log::warn!("[auth] keychain read failed ({e}); signed out");
                None
            }
        };
        Self {
            cfg,
            store,
            tokens: Arc::new(Mutex::new(tokens)),
            refresh_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    /// A valid access token, refreshing first when it expires within 60 s.
    /// `Ok(None)` when signed out (including a rejected refresh token).
    pub async fn access_token(&self) -> Result<Option<String>, AuthError> {
        match self.current() {
            None => return Ok(None),
            Some(t) if !needs_refresh(&t) => return Ok(Some(t.access_token)),
            Some(_) => {}
        }
        let _guard = self.refresh_lock.lock().await;
        // Another caller may have refreshed or signed out while we waited.
        let Some(old) = self.current() else {
            return Ok(None);
        };
        if !needs_refresh(&old) {
            return Ok(Some(old.access_token));
        }
        let Some(refresh_token) = old.refresh_token.clone() else {
            if old.expires_at > now_unix() {
                return Ok(Some(old.access_token));
            }
            log::info!("[auth] access token expired with no refresh token; signing out");
            return Ok(self.replace_if_current(&old, None));
        };
        self.refresh_locked(&old, &refresh_token).await
    }

    /// Refreshes regardless of expiry (e.g. after the server answered 401
    /// to a token we believed valid). Without a refresh token it returns the
    /// current token. `Ok(None)` when signed out or the refresh token is
    /// rejected.
    pub async fn force_refresh(&self) -> Result<Option<String>, AuthError> {
        let Some(before) = self.current() else {
            return Ok(None);
        };
        let _guard = self.refresh_lock.lock().await;
        let Some(old) = self.current() else {
            return Ok(None);
        };
        // Another caller refreshed (or signed in again) while we waited.
        if old != before {
            return Ok(Some(old.access_token));
        }
        let Some(refresh_token) = old.refresh_token.clone() else {
            return Ok(Some(old.access_token));
        };
        self.refresh_locked(&old, &refresh_token).await
    }

    /// Spends `refresh_token`; the caller holds `refresh_lock`.
    async fn refresh_locked(
        &self,
        old: &Tokens,
        refresh_token: &str,
    ) -> Result<Option<String>, AuthError> {
        match refresh_inner(&self.cfg, refresh_token).await {
            Ok(mut fresh) => {
                if fresh.id_token.is_none() {
                    fresh.id_token = old.id_token.clone();
                }
                Ok(self.replace_if_current(old, Some(fresh)))
            }
            Err(f) if f.oauth_error.as_deref() == Some("invalid_grant") => {
                log::info!("[auth] refresh token rejected; signing out");
                Ok(self.replace_if_current(old, None))
            }
            Err(f) => Err(f.err),
        }
    }

    /// Runs [`sign_in`] and stores the resulting tokens.
    pub async fn sign_in(
        &self,
        open: impl FnOnce(&str),
        timeout: Duration,
    ) -> Result<Account, AuthError> {
        let (tokens, account) = sign_in(&self.cfg, open, timeout).await?;
        let mut guard = self.tokens.lock().unwrap();
        self.persist(Some(&tokens));
        *guard = Some(tokens);
        Ok(account)
    }

    /// Forgets the session in memory and in the keychain.
    pub fn sign_out(&self) -> Result<(), String> {
        let mut guard = self.tokens.lock().unwrap();
        *guard = None;
        self.store.delete(CLERK_OAUTH_USER)
    }

    /// Display identity from the (unverified) id_token.
    pub fn account(&self) -> Option<Account> {
        self.current().as_ref().and_then(account_of)
    }

    pub fn is_signed_in(&self) -> bool {
        self.tokens.lock().unwrap().is_some()
    }

    fn current(&self) -> Option<Tokens> {
        self.tokens.lock().unwrap().clone()
    }

    /// Swaps in `next` only if the session still holds `expected` (so a
    /// sign-out or new sign-in during a refresh isn't overwritten). Returns
    /// the access token now in effect.
    fn replace_if_current(&self, expected: &Tokens, next: Option<Tokens>) -> Option<String> {
        let mut guard = self.tokens.lock().unwrap();
        if guard.as_ref() == Some(expected) {
            self.persist(next.as_ref());
            *guard = next;
        }
        guard.as_ref().map(|t| t.access_token.clone())
    }

    /// Best-effort keychain write; the in-memory session stays authoritative
    /// for this run if it fails.
    fn persist(&self, tokens: Option<&Tokens>) {
        let result = match tokens {
            Some(t) => serde_json::to_string(t)
                .map_err(|e| e.to_string())
                .and_then(|json| self.store.set(CLERK_OAUTH_USER, &json)),
            None => self.store.delete(CLERK_OAUTH_USER),
        };
        if let Err(e) = result {
            log::warn!("[auth] keychain write failed: {e}");
        }
    }
}

fn needs_refresh(t: &Tokens) -> bool {
    t.expires_at - now_unix() < REFRESH_MARGIN_SECS
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keychain::{MemKeyStore, CLERK_OAUTH_USER};
    use httpmock::prelude::*;
    use serde_json::json;
    use std::future::Future;

    fn cfg(frontend_api: &str, port: u16) -> AuthConfig {
        AuthConfig {
            frontend_api: frontend_api.to_string(),
            client_id: "client_123".to_string(),
            redirect_port: port,
        }
    }

    fn fake_jwt(payload: serde_json::Value) -> String {
        let h = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let p = URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("{h}.{p}.c2ln")
    }

    fn id_token() -> String {
        fake_jwt(json!({
            "sub": "user_1", "email": "a@b.co", "name": "Ada Lovelace",
            "picture": "https://img.example/a.png"
        }))
    }

    fn param(url: &str, key: &str) -> Option<String> {
        url::Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
    }

    fn now() -> i64 {
        now_unix()
    }

    /// Plays the browser: GET a loopback URL, return (status, body).
    async fn browser_get(url: &str) -> (u16, String) {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let resp = client.get(url).send().await.unwrap();
        let status = resp.status().as_u16();
        (status, resp.text().await.unwrap())
    }

    /// Runs `sign_in`, handing the authorize URL to `browser` concurrently.
    async fn run_sign_in<T, F, Fut>(
        cfg: &AuthConfig,
        timeout: Duration,
        browser: F,
    ) -> (Result<(Tokens, Account), AuthError>, Option<T>)
    where
        F: FnOnce(String) -> Fut,
        Fut: Future<Output = T>,
    {
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let flow = sign_in(cfg, move |url| tx.send(url.to_string()).unwrap(), timeout);
        let browse = async move {
            match rx.await {
                Ok(url) => Some(browser(url).await),
                Err(_) => None,
            }
        };
        tokio::join!(flow, browse)
    }

    fn token_json(access: &str, refresh: Option<&str>) -> serde_json::Value {
        json!({
            "access_token": access, "expires_in": 3600, "id_token": id_token(),
            "refresh_token": refresh, "scope": "openid profile email offline_access",
            "token_type": "Bearer"
        })
    }

    fn store_with(tokens: &Tokens) -> Arc<MemKeyStore> {
        let s = Arc::new(MemKeyStore::new());
        s.set(CLERK_OAUTH_USER, &serde_json::to_string(tokens).unwrap())
            .unwrap();
        s
    }

    fn stored(s: &MemKeyStore) -> Option<Tokens> {
        s.get(CLERK_OAUTH_USER)
            .unwrap()
            .map(|j| serde_json::from_str(&j).unwrap())
    }

    // ---- PKCE / config / URL ----

    #[test]
    fn pkce_rfc7636_vector() {
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn pkce_new_shape() {
        let a = Pkce::new();
        let b = Pkce::new();
        assert_eq!(a.verifier.len(), 64);
        assert!(a
            .verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_eq!(a.challenge, challenge_for(&a.verifier));
        assert!(a.state.len() >= 16);
        assert_ne!(a.verifier, b.verifier);
        assert_ne!(a.state, b.state);
        assert!(!format!("{a:?}").contains(&a.verifier));
    }

    #[test]
    fn authorize_url_has_params() {
        let c = cfg("https://clerk.example.com", DEFAULT_REDIRECT_PORT);
        let p = Pkce::new();
        let url = c.authorize_url(&p, DEFAULT_REDIRECT_PORT);
        assert!(
            url.starts_with("https://clerk.example.com/oauth/authorize?"),
            "{url}"
        );
        for needle in [
            "response_type=code",
            "code_challenge_method=S256",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A17823%2Fcallback",
            "scope=openid+profile+email+offline_access",
            "client_id=client_123",
        ] {
            assert!(url.contains(needle), "missing {needle} in {url}");
        }
        assert_eq!(param(&url, "state").as_deref(), Some(p.state.as_str()));
        assert_eq!(
            param(&url, "code_challenge").as_deref(),
            Some(p.challenge.as_str())
        );
        assert!(!url.contains(&p.verifier));
    }

    #[test]
    fn config_from_values() {
        let c = AuthConfig::from_values(
            Some("https://clerk.example.com/".into()),
            Some("cid".into()),
        )
        .unwrap();
        assert_eq!(c.frontend_api, "https://clerk.example.com");
        assert_eq!(c.client_id, "cid");
        assert_eq!(c.redirect_port, DEFAULT_REDIRECT_PORT);
        assert!(AuthConfig::from_values(None, Some("cid".into())).is_none());
        assert!(AuthConfig::from_values(Some("https://x".into()), Some("  ".into())).is_none());
        assert!(AuthConfig::from_values(Some("".into()), Some("cid".into())).is_none());
    }

    #[test]
    fn env_value_prefers_runtime_then_compiled() {
        assert_eq!(
            env_value(Some("rt".into()), Some("ct")).as_deref(),
            Some("rt")
        );
        assert_eq!(
            env_value(Some("".into()), Some("ct")).as_deref(),
            Some("ct")
        );
        assert_eq!(env_value(None, Some("ct")).as_deref(), Some("ct"));
        assert_eq!(env_value(None, None), None);
    }

    // ---- id_token ----

    #[test]
    fn account_from_id_token() {
        let acct = super::account_from_id_token(&id_token()).unwrap();
        assert_eq!(
            acct,
            Account {
                id: "user_1".into(),
                email: "a@b.co".into(),
                name: "Ada Lovelace".into(),
                image_url: Some("https://img.example/a.png".into()),
            }
        );
        let bare = fake_jwt(json!({"sub": "user_2"}));
        let acct = super::account_from_id_token(&bare).unwrap();
        assert_eq!(
            (acct.email.as_str(), acct.name.as_str(), acct.image_url),
            ("", "", None)
        );
        assert!(super::account_from_id_token(&fake_jwt(json!({"email": "x"}))).is_none());
        assert!(super::account_from_id_token("not-a-jwt").is_none());
        assert!(super::account_from_id_token("a.%%%.c").is_none());
    }

    #[test]
    fn secrets_not_in_debug() {
        let t = Tokens {
            access_token: "at-secret".into(),
            refresh_token: Some("rt-secret".into()),
            expires_at: 1,
            id_token: Some("idt-secret".into()),
        };
        let d = format!("{t:?}");
        assert!(!d.contains("secret"), "{d}");
    }

    // ---- loopback sign-in ----

    #[tokio::test]
    async fn callback_exchanges_code() {
        let server = MockServer::start_async().await;
        let c = cfg(&server.base_url(), 0);
        let (res, mock) = run_sign_in(&c, Duration::from_secs(10), |url| {
            let server = &server;
            async move {
                let challenge = param(&url, "code_challenge").unwrap();
                let redirect = param(&url, "redirect_uri").unwrap();
                let state = param(&url, "state").unwrap();
                assert!(redirect.starts_with("http://127.0.0.1:"));
                assert!(!redirect.ends_with(":0/callback"));
                let redirect_m = redirect.clone();
                let mock = server
                    .mock_async(move |when, then| {
                        when.method(POST)
                            .path("/oauth/token")
                            .header("content-type", "application/x-www-form-urlencoded")
                            .form_urlencoded_tuple("grant_type", "authorization_code")
                            .form_urlencoded_tuple("code", "the-code")
                            .form_urlencoded_tuple("client_id", "client_123")
                            .form_urlencoded_tuple("redirect_uri", redirect_m)
                            .form_urlencoded_tuple_exists("code_verifier")
                            .form_urlencoded_tuple_missing("client_secret")
                            .is_true(move |req| {
                                url::form_urlencoded::parse(req.body().as_ref())
                                    .find(|(k, _)| k == "code_verifier")
                                    .is_some_and(|(_, v)| challenge_for(&v) == challenge)
                            });
                        then.status(200).json_body(token_json("at-1", Some("rt-1")));
                    })
                    .await;
                // Browsers ask for a favicon; it must not end the flow.
                let host = redirect.trim_end_matches("/callback");
                let (st, _) = browser_get(&format!("{host}/favicon.ico")).await;
                assert_eq!(st, 404);
                let (st, body) =
                    browser_get(&format!("{redirect}?code=the-code&state={state}")).await;
                assert_eq!(st, 200);
                assert!(body.contains("You can close this tab"), "{body}");
                mock
            }
        })
        .await;
        let (tokens, acct) = res.unwrap();
        mock.unwrap().assert_async().await;
        assert_eq!(tokens.access_token, "at-1");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt-1"));
        assert!((tokens.expires_at - (now() + 3600)).abs() <= 5);
        assert_eq!(acct.id, "user_1");
        assert_eq!(acct.email, "a@b.co");
    }

    #[tokio::test]
    async fn callback_state_mismatch_errors() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.path("/oauth/token");
                then.status(200).json_body(token_json("at-1", None));
            })
            .await;
        let c = cfg(&server.base_url(), 0);
        let (res, _) = run_sign_in(&c, Duration::from_secs(10), |url| async move {
            let redirect = param(&url, "redirect_uri").unwrap();
            let (st, body) = browser_get(&format!("{redirect}?code=x&state=forged")).await;
            assert_ne!(st, 200);
            assert!(!body.contains("forged"));
        })
        .await;
        assert_eq!(res.unwrap_err(), AuthError::StateMismatch);
        mock.assert_calls_async(0).await;
    }

    #[tokio::test]
    async fn callback_error_param_is_denied() {
        let c = cfg("http://127.0.0.1:9", 0);
        let (res, _) = run_sign_in(&c, Duration::from_secs(10), |url| async move {
            let redirect = param(&url, "redirect_uri").unwrap();
            let state = param(&url, "state").unwrap();
            browser_get(&format!("{redirect}?error=access_denied&state={state}")).await;
        })
        .await;
        assert_eq!(res.unwrap_err(), AuthError::Denied("access_denied".into()));
    }

    #[tokio::test]
    async fn callback_times_out() {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let c = cfg("http://127.0.0.1:9", port);
        let start = std::time::Instant::now();
        let (res, _) = run_sign_in(&c, Duration::from_millis(200), |_url| async {}).await;
        assert_eq!(res.unwrap_err(), AuthError::Timeout);
        assert!(start.elapsed() < Duration::from_secs(5));
        // The listener is closed afterwards: the port can be bound again.
        std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    }

    #[tokio::test]
    async fn port_in_use_errors() {
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let c = cfg("http://127.0.0.1:9", held.local_addr().unwrap().port());
        let mut opened = false;
        let res = sign_in(&c, |_| opened = true, Duration::from_secs(1)).await;
        assert_eq!(res.unwrap_err(), AuthError::PortInUse);
        assert!(!opened, "browser must not open when the port is taken");
    }

    // ---- refresh / session ----

    #[tokio::test]
    async fn refresh_keeps_refresh_token_when_absent() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/oauth/token")
                    .form_urlencoded_tuple("grant_type", "refresh_token")
                    .form_urlencoded_tuple("refresh_token", "rt-old")
                    .form_urlencoded_tuple("client_id", "client_123")
                    .form_urlencoded_tuple_missing("client_secret");
                then.status(200)
                    .json_body(json!({"access_token": "at-2", "expires_in": 60}));
            })
            .await;
        let t = refresh(&cfg(&server.base_url(), 0), "rt-old")
            .await
            .unwrap();
        assert_eq!(t.access_token, "at-2");
        assert_eq!(t.refresh_token.as_deref(), Some("rt-old"));
        assert!((t.expires_at - (now() + 60)).abs() <= 5);
    }

    #[tokio::test]
    async fn http_error_is_bounded_and_secret_free() {
        let server = MockServer::start_async().await;
        let long = "x".repeat(5000);
        server
            .mock_async(move |when, then| {
                when.path("/oauth/token");
                then.status(502).body(long);
            })
            .await;
        let err = refresh(&cfg(&server.base_url(), 0), "rt-secret")
            .await
            .unwrap_err();
        let AuthError::Http(msg) = &err else {
            panic!("{err:?}")
        };
        assert!(msg.contains("502"), "{msg}");
        assert!(msg.len() < 400, "{}", msg.len());
        assert!(!format!("{err} {err:?}").contains("rt-secret"));
    }

    fn near_expiry_tokens() -> Tokens {
        Tokens {
            access_token: "at-old".into(),
            refresh_token: Some("rt-old".into()),
            expires_at: now() + 30,
            id_token: Some(id_token()),
        }
    }

    #[tokio::test]
    async fn session_refreshes_when_near_expiry() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/oauth/token")
                    .form_urlencoded_tuple("grant_type", "refresh_token")
                    .form_urlencoded_tuple("refresh_token", "rt-old");
                then.status(200).json_body(json!({
                    "access_token": "at-new", "expires_in": 3600, "refresh_token": "rt-new"
                }));
            })
            .await;
        let store = store_with(&near_expiry_tokens());
        let s = Session::load(cfg(&server.base_url(), 0), store.clone());
        assert_eq!(s.access_token().await.unwrap().as_deref(), Some("at-new"));
        assert_eq!(s.access_token().await.unwrap().as_deref(), Some("at-new"));
        mock.assert_calls_async(1).await;
        let persisted = stored(&store).unwrap();
        assert_eq!(persisted.access_token, "at-new");
        assert_eq!(persisted.refresh_token.as_deref(), Some("rt-new"));
        // id_token survives a refresh that doesn't return one.
        assert_eq!(persisted.id_token, Some(id_token()));
        assert_eq!(s.account().unwrap().id, "user_1");
    }

    #[tokio::test]
    async fn session_concurrent_callers_refresh_once() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.path("/oauth/token");
                then.status(200)
                    .delay(Duration::from_millis(150))
                    .json_body(json!({"access_token": "at-new", "expires_in": 3600}));
            })
            .await;
        let s = Session::load(
            cfg(&server.base_url(), 0),
            store_with(&near_expiry_tokens()),
        );
        let s2 = s.clone();
        let (a, b) = tokio::join!(s.access_token(), s2.access_token());
        assert_eq!(a.unwrap().as_deref(), Some("at-new"));
        assert_eq!(b.unwrap().as_deref(), Some("at-new"));
        mock.assert_calls_async(1).await;
    }

    #[tokio::test]
    async fn session_fresh_token_skips_refresh() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.path("/oauth/token");
                then.status(500);
            })
            .await;
        let mut t = near_expiry_tokens();
        t.expires_at = now() + 3600;
        let s = Session::load(cfg(&server.base_url(), 0), store_with(&t));
        assert_eq!(s.access_token().await.unwrap().as_deref(), Some("at-old"));
        mock.assert_calls_async(0).await;
    }

    #[tokio::test]
    async fn force_refresh_ignores_expiry() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(POST)
                    .path("/oauth/token")
                    .form_urlencoded_tuple("grant_type", "refresh_token")
                    .form_urlencoded_tuple("refresh_token", "rt-old");
                then.status(200).json_body(json!({
                    "access_token": "at-new", "expires_in": 3600, "refresh_token": "rt-new"
                }));
            })
            .await;
        let mut t = near_expiry_tokens();
        t.expires_at = now() + 3600;
        let store = store_with(&t);
        let s = Session::load(cfg(&server.base_url(), 0), store.clone());
        assert_eq!(s.force_refresh().await.unwrap().as_deref(), Some("at-new"));
        mock.assert_calls_async(1).await;
        assert_eq!(
            stored(&store).unwrap().refresh_token.as_deref(),
            Some("rt-new")
        );
        assert_eq!(s.access_token().await.unwrap().as_deref(), Some("at-new"));
        assert_eq!(s.account().unwrap().id, "user_1");

        // No refresh token: the current token comes back, no request.
        let mut bare = t.clone();
        bare.refresh_token = None;
        let s2 = Session::load(cfg(&server.base_url(), 0), store_with(&bare));
        assert_eq!(s2.force_refresh().await.unwrap().as_deref(), Some("at-old"));
        mock.assert_calls_async(1).await;

        // Signed out stays signed out.
        let s3 = Session::load(cfg(&server.base_url(), 0), Arc::new(MemKeyStore::new()));
        assert_eq!(s3.force_refresh().await.unwrap(), None);
    }

    #[tokio::test]
    async fn force_refresh_invalid_grant_signs_out() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/oauth/token");
                then.status(400)
                    .json_body(json!({"error": "invalid_grant"}));
            })
            .await;
        let mut t = near_expiry_tokens();
        t.expires_at = now() + 3600;
        let store = store_with(&t);
        let s = Session::load(cfg(&server.base_url(), 0), store.clone());
        assert_eq!(s.force_refresh().await.unwrap(), None);
        assert!(!s.is_signed_in());
        assert!(stored(&store).is_none());
    }

    #[tokio::test]
    async fn session_invalid_grant_signs_out() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.path("/oauth/token");
                then.status(400)
                    .json_body(json!({"error": "invalid_grant", "error_description": "expired"}));
            })
            .await;
        let store = store_with(&near_expiry_tokens());
        let s = Session::load(cfg(&server.base_url(), 0), store.clone());
        assert_eq!(s.access_token().await.unwrap(), None);
        assert!(stored(&store).is_none());
        assert!(s.account().is_none());
    }

    #[tokio::test]
    async fn session_other_refresh_error_keeps_tokens() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.path("/oauth/token");
                then.status(503).body("unavailable");
            })
            .await;
        let store = store_with(&near_expiry_tokens());
        let s = Session::load(cfg(&server.base_url(), 0), store.clone());
        assert!(matches!(s.access_token().await, Err(AuthError::Http(_))));
        assert_eq!(stored(&store).unwrap().access_token, "at-old");
        assert!(s.account().is_some());
    }

    #[tokio::test]
    async fn session_signed_out_and_sign_out() {
        let empty = Session::load(cfg("http://127.0.0.1:9", 0), Arc::new(MemKeyStore::new()));
        assert_eq!(empty.access_token().await.unwrap(), None);
        assert!(empty.account().is_none());

        let corrupt = Arc::new(MemKeyStore::new());
        corrupt.set(CLERK_OAUTH_USER, "{not json").unwrap();
        let s = Session::load(cfg("http://127.0.0.1:9", 0), corrupt);
        assert_eq!(s.access_token().await.unwrap(), None);

        let mut t = near_expiry_tokens();
        t.expires_at = now() + 3600;
        let store = store_with(&t);
        let s = Session::load(cfg("http://127.0.0.1:9", 0), store.clone());
        assert!(s.is_signed_in());
        s.sign_out().unwrap();
        assert!(!s.is_signed_in());
        assert!(stored(&store).is_none());
        assert_eq!(s.access_token().await.unwrap(), None);
    }

    #[tokio::test]
    async fn session_expired_without_refresh_token_is_signed_out() {
        let t = Tokens {
            access_token: "at-old".into(),
            refresh_token: None,
            expires_at: now() - 10,
            id_token: None,
        };
        let store = store_with(&t);
        let s = Session::load(cfg("http://127.0.0.1:9", 0), store.clone());
        assert_eq!(s.access_token().await.unwrap(), None);
        assert!(stored(&store).is_none());
    }

    #[tokio::test]
    async fn session_sign_in_persists_tokens() {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.path("/oauth/token");
                then.status(200).json_body(token_json("at-1", Some("rt-1")));
            })
            .await;
        let store = Arc::new(MemKeyStore::new());
        let s = Session::load(cfg(&server.base_url(), 0), store.clone());
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let flow = s.sign_in(
            move |u| tx.send(u.to_string()).unwrap(),
            Duration::from_secs(10),
        );
        let browse = async {
            let url = rx.await.unwrap();
            let redirect = param(&url, "redirect_uri").unwrap();
            let state = param(&url, "state").unwrap();
            browser_get(&format!("{redirect}?code=c&state={state}")).await;
        };
        let (acct, ()) = tokio::join!(flow, browse);
        assert_eq!(acct.unwrap().id, "user_1");
        assert_eq!(stored(&store).unwrap().access_token, "at-1");
        assert_eq!(s.access_token().await.unwrap().as_deref(), Some("at-1"));
    }
}
