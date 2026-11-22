//! The [`Client`], its [`ClientBuilder`] and the request engine behind every call.

use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use reqwest::Method;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::{Error, ErrorBody, Result};
use crate::resources::{Dubs, LanguagesApi, Runs, Uploads, UsageApi, WebhookEndpoints};
use crate::types::{CreateDubParams, DubParams, DubsCreated};

/// The API's address, unless you change it.
pub const DEFAULT_BASE_URL: &str = "https://api.reedapt.com/v1";

/// The version of this crate, sent in the `User-Agent` header.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_MAX_RETRIES: u32 = 2;

/// Settings for one call or a group of calls. Apply them with [`Client::with_options`].
///
/// ```
/// # use std::time::Duration;
/// use reedapt::RequestOptions;
///
/// let client = reedapt::Client::new("rk_test_example");
/// let patient = client.with_options(RequestOptions::new().timeout(Duration::from_secs(5)).max_retries(0));
/// ```
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct RequestOptions {
    /// Sent as `Idempotency-Key`. Creating uploads and dubs gets one automatically.
    pub idempotency_key: Option<String>,
    /// Overrides the client's timeout.
    pub timeout: Option<Duration>,
    /// Overrides the client's retry count.
    pub max_retries: Option<u32>,
}

impl RequestOptions {
    /// No overrides.
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes a repeat of the call safe, even across restarts: the API answers a repeated
    /// key with the first call's result. Use one key per intended action, such as
    /// `order-1042`.
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// How long one attempt may take.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// How many times to retry a call that failed in a way worth retrying.
    pub fn max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = Some(max_retries);
        self
    }
}

/// Whether a request may be sent again after any failure.
///
/// `Safe` requests can be repeated without doing the work twice. `Unsafe` ones are only
/// retried when the API refused them before acting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Retry {
    Safe,
    Unsafe,
}

pub(crate) struct Req {
    pub method: Method,
    pub path: String,
    pub query: Vec<(&'static str, String)>,
    pub body: Option<Vec<u8>>,
    pub retry: Retry,
    /// Gets a generated `Idempotency-Key` when the caller didn't give one.
    pub idempotent: bool,
}

impl Req {
    pub fn get(path: impl Into<String>) -> Self {
        Self::new(Method::GET, path, Retry::Safe)
    }

    pub fn post(path: impl Into<String>, retry: Retry) -> Self {
        Self::new(Method::POST, path, retry)
    }

    fn new(method: Method, path: impl Into<String>, retry: Retry) -> Self {
        Self {
            method,
            path: path.into(),
            query: Vec::new(),
            body: None,
            retry,
            idempotent: false,
        }
    }

    pub fn delete(path: impl Into<String>) -> Self {
        Self::new(Method::DELETE, path, Retry::Safe)
    }

    pub fn query(mut self, name: &'static str, value: impl ToString) -> Self {
        self.query.push((name, value.to_string()));
        self
    }

    pub fn json(mut self, body: &impl Serialize) -> Result<Self> {
        self.body = Some(serde_json::to_vec(body).map_err(Error::Decode)?);
        Ok(self)
    }

    pub fn idempotent(mut self) -> Self {
        self.idempotent = true;
        self
    }
}

/// Builds a [`Client`].
///
/// ```
/// use std::time::Duration;
///
/// let client = reedapt::Client::builder()
///     .api_key("rk_test_example")
///     .timeout(Duration::from_secs(30))
///     .max_retries(3)
///     .build()?;
/// # Ok::<(), reedapt::Error>(())
/// ```
#[derive(Default)]
pub struct ClientBuilder {
    api_key: Option<String>,
    base_url: Option<String>,
    timeout: Option<Duration>,
    max_retries: Option<u32>,
    http: Option<reqwest::Client>,
    headers: HeaderMap,
}

impl ClientBuilder {
    /// The API key. Defaults to the `REEDAPT_API_KEY` environment variable.
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Defaults to `REEDAPT_BASE_URL`, then `https://api.reedapt.com/v1`.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// How long one attempt may take. Defaults to 60 seconds.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// How many times to retry a call that failed in a way worth retrying. Defaults to 2.
    pub fn max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = Some(max_retries);
        self
    }

    /// Uses your own `reqwest::Client`, for a proxy, extra root certificates or shared
    /// connection pools. Its own timeout is left alone; the client's per-request timeout
    /// still applies.
    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http = Some(client);
        self
    }

    /// Adds a header to every request. Panics if the name or value isn't a valid header.
    pub fn default_header(mut self, name: &str, value: &str) -> Self {
        let name = HeaderName::from_bytes(name.as_bytes()).expect("invalid header name");
        let value = HeaderValue::from_str(value).expect("invalid header value");
        self.headers.insert(name, value);
        self
    }

    /// Builds the client. Fails if there is no API key.
    pub fn build(self) -> Result<Client> {
        let api_key = self
            .api_key
            .or_else(|| env("REEDAPT_API_KEY"))
            .ok_or_else(|| {
                Error::Config(
                    "No API key. Pass one to Client::new or the builder, or set the REEDAPT_API_KEY environment variable."
                        .into(),
                )
            })?;
        let mut auth = HeaderValue::from_str(&format!("Bearer {api_key}")).map_err(|_| {
            Error::Config("The API key contains characters that can't be sent in a header.".into())
        })?;
        auth.set_sensitive(true);
        let base_url = self
            .base_url
            .or_else(|| env("REEDAPT_BASE_URL"))
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        let http = match self.http {
            Some(http) => http,
            None => reqwest::Client::builder().build().map_err(Error::Transport)?,
        };
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&format!("reedapt-rust/{VERSION}")).expect("valid user agent"),
        );
        headers.extend(self.headers);
        headers.insert(AUTHORIZATION, auth);
        Ok(Client {
            inner: Arc::new(Inner {
                http,
                base_url: base_url.trim_end_matches('/').to_string(),
                headers,
                timeout: self.timeout.unwrap_or(DEFAULT_TIMEOUT),
                max_retries: self.max_retries.unwrap_or(DEFAULT_MAX_RETRIES),
            }),
            options: RequestOptions::default(),
        })
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

struct Inner {
    http: reqwest::Client,
    base_url: String,
    headers: HeaderMap,
    timeout: Duration,
    max_retries: u32,
}

/// The entry point: one client talks to the whole API. It is cheap to clone and safe to
/// share between tasks; clones share one connection pool.
///
/// ```no_run
/// use reedapt::types::{CreateDubParams, CreateUploadParams};
///
/// # async fn run() -> reedapt::Result<()> {
/// let client = reedapt::Client::new(std::env::var("REEDAPT_API_KEY").unwrap());
///
/// let upload = client.uploads().upload_file("interview.mp4").await?;
/// let created = client
///     .dubs()
///     .create(CreateDubParams::new(upload.id, "en", ["fr", "es"]))
///     .await?;
/// for dub in created.dubs {
///     let dub = client.dubs().wait_until_done(&dub.id, Default::default()).await?;
///     println!("{:?} {:?}", dub.target_language, dub.status);
/// }
/// # Ok(()) }
/// ```
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
    options: RequestOptions,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.inner.base_url)
            .field("timeout", &self.inner.timeout)
            .field("max_retries", &self.inner.max_retries)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client for the production API.
    ///
    /// # Panics
    ///
    /// If the TLS backend can't be initialised or the key can't be sent in a header. Use
    /// [`Client::builder`] to handle those as errors.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::builder()
            .api_key(api_key)
            .build()
            .expect("could not build the Reedapt client")
    }

    /// A client configured from `REEDAPT_API_KEY` and, if set, `REEDAPT_BASE_URL`.
    pub fn from_env() -> Result<Self> {
        Self::builder().build()
    }

    /// Starts building a client with a custom base URL, timeout, retry count or HTTP client.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// A copy of this client whose calls use `options`. Handy for one call:
    ///
    /// ```no_run
    /// # async fn run(client: reedapt::Client, params: reedapt::types::CreateDubParams) -> reedapt::Result<()> {
    /// let created = client
    ///     .with_options(reedapt::RequestOptions::new().idempotency_key("order-1042"))
    ///     .dubs()
    ///     .create(params)
    ///     .await?;
    /// # Ok(()) }
    /// ```
    pub fn with_options(&self, options: RequestOptions) -> Client {
        let mut merged = self.options.clone();
        if options.idempotency_key.is_some() {
            merged.idempotency_key = options.idempotency_key;
        }
        if options.timeout.is_some() {
            merged.timeout = options.timeout;
        }
        if options.max_retries.is_some() {
            merged.max_retries = options.max_retries;
        }
        Client {
            inner: self.inner.clone(),
            options: merged,
        }
    }

    /// This client's idempotency key, if `with_options` set one.
    pub(crate) fn idempotency_key(&self) -> Option<&str> {
        self.options.idempotency_key.as_deref()
    }

    /// A copy that sends `key` as its idempotency key (or none).
    pub(crate) fn with_idempotency_key(&self, key: Option<String>) -> Client {
        let mut options = self.options.clone();
        options.idempotency_key = key;
        Client {
            inner: self.inner.clone(),
            options,
        }
    }

    /// The base URL requests go to, without a trailing slash.
    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    /// Uploads: sending files in parts or having Reedapt fetch a link.
    pub fn uploads(&self) -> Uploads {
        Uploads::new(self.clone())
    }

    /// Dubs: create, read, list, cancel, download, transcribe and check.
    pub fn dubs(&self) -> Dubs {
        Dubs::new(self.clone())
    }

    /// Requests for changes to dubs. Early access.
    pub fn runs(&self) -> Runs {
        Runs::new(self.clone())
    }

    /// Webhook endpoints and their deliveries.
    pub fn webhook_endpoints(&self) -> WebhookEndpoints {
        WebhookEndpoints::new(self.clone())
    }

    /// The languages dubs can be made in.
    pub fn languages(&self) -> LanguagesApi {
        LanguagesApi::new(self.clone())
    }

    /// What the key can do and what it has used.
    pub fn usage(&self) -> UsageApi {
        UsageApi::new(self.clone())
    }

    /// Sends a file from disk and starts a dub for each target language. Resolves once
    /// the dubs have started, not when they finish.
    ///
    /// With an idempotency key set through [`Client::with_options`], the upload and the
    /// dubs each get that key with a suffix, so the whole call is safe to repeat.
    pub async fn dub_file(&self, path: impl AsRef<Path>, params: DubParams) -> Result<DubsCreated> {
        let upload = self
            .with_idempotency_key(self.suffixed_key("upload"))
            .uploads()
            .upload_file(path)
            .await?;
        self.create_dubs_for(upload.id, params).await
    }

    /// Has Reedapt download a public link and starts a dub for each target language.
    /// Resolves once the dubs have started, not when they finish.
    pub async fn dub_url(&self, url: impl Into<String>, params: DubParams) -> Result<DubsCreated> {
        let upload = self
            .with_idempotency_key(self.suffixed_key("upload"))
            .uploads()
            .from_url(url, Default::default())
            .await?;
        self.create_dubs_for(upload.id, params).await
    }

    fn suffixed_key(&self, suffix: &str) -> Option<String> {
        self.idempotency_key().map(|k| format!("{k}:{suffix}"))
    }

    async fn create_dubs_for(&self, upload_id: String, params: DubParams) -> Result<DubsCreated> {
        self.with_idempotency_key(self.suffixed_key("dubs"))
            .dubs()
            .create(CreateDubParams::with_options(upload_id, params))
            .await
    }

    // ------------------------------------------------------------ engine

    pub(crate) async fn json<T: DeserializeOwned>(&self, req: Req) -> Result<T> {
        let bytes = self.send(req).await?;
        serde_json::from_slice(&bytes).map_err(Error::Decode)
    }

    pub(crate) async fn empty(&self, req: Req) -> Result<()> {
        self.send(req).await.map(|_| ())
    }

    pub(crate) async fn text(&self, req: Req) -> Result<String> {
        let bytes = self.send(req).await?;
        String::from_utf8(bytes.to_vec())
            .map_err(|e| Error::Upload(format!("the API's answer was not text: {e}")))
    }

    /// The client's HTTP stack, for sending parts to storage without the API key.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.inner.http
    }

    pub(crate) fn timeout(&self) -> Duration {
        self.options.timeout.unwrap_or(self.inner.timeout)
    }

    async fn send(&self, req: Req) -> Result<bytes::Bytes> {
        let url = format!("{}{}", self.inner.base_url, req.path);
        let idempotency_key = self
            .options
            .idempotency_key
            .clone()
            .filter(|_| req.method != Method::GET)
            .or_else(|| req.idempotent.then(|| uuid::Uuid::new_v4().to_string()));
        // A request carrying an idempotency key is safe to send again.
        let retry = if idempotency_key.is_some() {
            Retry::Safe
        } else {
            req.retry
        };
        let max_retries = self.options.max_retries.unwrap_or(self.inner.max_retries);

        let mut attempt = 0;
        loop {
            let mut builder = self
                .inner
                .http
                .request(req.method.clone(), &url)
                .headers(self.inner.headers.clone())
                .timeout(self.timeout());
            if !req.query.is_empty() {
                builder = builder.query(&req.query);
            }
            if let Some(key) = &idempotency_key {
                builder = builder.header("Idempotency-Key", key);
            }
            if let Some(body) = &req.body {
                builder = builder
                    .header(CONTENT_TYPE, "application/json")
                    .body(body.clone());
            }

            let (error, retry_after) = match builder.send().await {
                Ok(res) => {
                    let status = res.status();
                    let retry_after = parse_retry_after(res.headers());
                    let request_id = res
                        .headers()
                        .get("request-id")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned);
                    match res.bytes().await {
                        Ok(bytes) if status.is_success() => return Ok(bytes),
                        Ok(bytes) => {
                            let body = serde_json::from_slice::<ErrorEnvelope>(&bytes)
                                .ok()
                                .and_then(|e| e.error);
                            (
                                Error::from_response(status.as_u16(), body, request_id, retry_after),
                                retry_after,
                            )
                        }
                        Err(e) => (Error::Transport(e), None),
                    }
                }
                Err(e) => (Error::Transport(e), None),
            };

            if attempt >= max_retries || !should_retry(&error, retry) {
                return Err(error);
            }
            tokio::time::sleep(retry_delay(retry_after, attempt)).await;
            attempt += 1;
        }
    }
}

#[derive(serde::Deserialize)]
struct ErrorEnvelope {
    error: Option<ErrorBody>,
}

/// `Retry-After` in seconds. Dates aren't used by the API.
fn parse_retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds: f64 = headers.get("retry-after")?.to_str().ok()?.trim().parse().ok()?;
    (seconds.is_finite() && seconds >= 0.0).then(|| Duration::from_secs_f64(seconds))
}

pub(crate) fn should_retry(error: &Error, retry: Retry) -> bool {
    match error {
        Error::Transport(_) => retry == Retry::Safe,
        // Some 429s are limits that last minutes or hours, not a busy minute.
        Error::RateLimited { code, .. } => code == "RATE_LIMITED",
        Error::Api { status: 503, .. } => true,
        _ if retry == Retry::Unsafe => false,
        Error::Api {
            status: 409, code, ..
        } => code == "IDEMPOTENCY_IN_PROGRESS",
        Error::Api { status, .. } => matches!(status, 408 | 500 | 502 | 504),
        _ => false,
    }
}

/// How long to wait before the next attempt: the API's `Retry-After` when it is a minute
/// or less, otherwise exponential backoff with jitter.
pub(crate) fn retry_delay(retry_after: Option<Duration>, attempt: u32) -> Duration {
    match retry_after {
        Some(d) if d <= Duration::from_secs(60) => d,
        _ => backoff(attempt),
    }
}

/// 0.5 s, 1 s, 2 s ... up to 8 s, each scaled by 0.75 to 1.0.
pub(crate) fn backoff(attempt: u32) -> Duration {
    let base = (500u64 << attempt.min(10)).min(8000);
    let jitter = 0.75 + 0.25 * f64::from(uuid::Uuid::new_v4().as_bytes()[0]) / 255.0;
    Duration::from_millis((base as f64 * jitter) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(status: u16, code: &str) -> Error {
        Error::from_response(
            status,
            Some(ErrorBody {
                code: Some(code.into()),
                ..ErrorBody::default()
            }),
            None,
            None,
        )
    }

    #[test]
    fn which_failures_are_retried() {
        use Retry::{Safe, Unsafe};
        assert!(should_retry(&api(429, "RATE_LIMITED"), Unsafe));
        assert!(!should_retry(&api(429, "TOO_MANY_DUBS_RUNNING"), Safe));
        assert!(should_retry(&api(503, "X"), Unsafe));
        for status in [408, 500, 502, 504] {
            assert!(should_retry(&api(status, "X"), Safe), "{status}");
            assert!(!should_retry(&api(status, "X"), Unsafe), "{status}");
        }
        assert!(should_retry(&api(409, "IDEMPOTENCY_IN_PROGRESS"), Safe));
        assert!(!should_retry(&api(409, "UPLOAD_COMPLETE"), Safe));
        for status in [400, 401, 404, 422] {
            assert!(!should_retry(&api(status, "X"), Safe), "{status}");
        }
    }

    #[test]
    fn retry_after_wins_over_backoff_up_to_a_minute() {
        assert_eq!(
            retry_delay(Some(Duration::from_secs(7)), 0),
            Duration::from_secs(7)
        );
        assert_eq!(retry_delay(Some(Duration::ZERO), 3), Duration::ZERO);
        let long = retry_delay(Some(Duration::from_secs(61)), 0);
        assert!(long <= Duration::from_millis(500), "{long:?}");
    }

    #[test]
    fn backoff_doubles_with_jitter_and_caps_at_eight_seconds() {
        for attempt in 0..12 {
            let base = (500u64 << attempt.min(10)).min(8000) as f64;
            let ms = backoff(attempt).as_millis() as f64;
            assert!(
                ms >= base * 0.75 - 1.0 && ms <= base,
                "attempt {attempt}: {ms} vs {base}"
            );
        }
    }

    #[test]
    fn retry_after_parses_seconds_only() {
        let mut h = HeaderMap::new();
        h.insert("retry-after", "2.5".parse().unwrap());
        assert_eq!(parse_retry_after(&h), Some(Duration::from_millis(2500)));
        h.insert("retry-after", "Wed, 21 Oct 2026 07:28:00 GMT".parse().unwrap());
        assert_eq!(parse_retry_after(&h), None);
        h.insert("retry-after", "-1".parse().unwrap());
        assert_eq!(parse_retry_after(&h), None);
    }
}
