//! A blocking client, for programs that don't use async. Enable the `blocking` feature.
//!
//! It wraps the async [`Client`](crate::Client) with a small private runtime and has the
//! same resources and methods without `.await`:
//!
//! ```no_run
//! use reedapt::types::{CreateDubParams, DubParams};
//!
//! # fn main() -> reedapt::Result<()> {
//! let client = reedapt::blocking::Client::new("rk_test_example");
//! let created = client.dub_file("interview.mp4", DubParams::new("en", ["fr"]))?;
//! let dub = client.dubs().wait_until_done(&created.dubs[0].id, Default::default())?;
//! println!("{:?}", dub.status);
//!
//! for dub in client.dubs().list_all(None) {
//!     println!("{}", dub?.id);
//! }
//! # Ok(()) }
//! ```
//!
//! Don't call it from inside an async runtime: it blocks the thread, and tokio panics if
//! you try. Use the async client there.

use std::collections::VecDeque;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use bytes::Bytes;

use crate::error::{Error, Result};
use crate::resources::{FromUrlOptions, UploadOptions, WaitForDubOptions, WaitOptions};
use crate::types::*;
use crate::RequestOptions;

/// The blocking [`ClientBuilder`](crate::ClientBuilder).
#[derive(Default)]
pub struct ClientBuilder(crate::ClientBuilder);

impl ClientBuilder {
    /// The API key. Defaults to the `REEDAPT_API_KEY` environment variable.
    pub fn api_key(self, api_key: impl Into<String>) -> Self {
        Self(self.0.api_key(api_key))
    }

    /// Defaults to `REEDAPT_BASE_URL`, then `https://api.reedapt.com/v1`.
    pub fn base_url(self, base_url: impl Into<String>) -> Self {
        Self(self.0.base_url(base_url))
    }

    /// How long one attempt may take. Defaults to 60 seconds.
    pub fn timeout(self, timeout: std::time::Duration) -> Self {
        Self(self.0.timeout(timeout))
    }

    /// How many times to retry a call that failed in a way worth retrying. Defaults to 2.
    pub fn max_retries(self, max_retries: u32) -> Self {
        Self(self.0.max_retries(max_retries))
    }

    /// Uses your own async `reqwest::Client`.
    pub fn http_client(self, client: reqwest::Client) -> Self {
        Self(self.0.http_client(client))
    }

    /// Builds the client. Fails if there is no API key.
    pub fn build(self) -> Result<Client> {
        Client::from_async(self.0.build()?)
    }
}

/// The blocking client. Cheap to clone; clones share one runtime and connection pool.
#[derive(Clone)]
pub struct Client {
    inner: crate::Client,
    rt: Arc<tokio::runtime::Runtime>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("blocking::Client")
            .field("inner", &self.inner)
            .finish()
    }
}

impl Client {
    /// A client for the production API.
    ///
    /// # Panics
    ///
    /// If the runtime or TLS backend can't be initialised. Use [`Client::builder`] to
    /// handle that as an error.
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

    /// Starts building a client.
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// Wraps an async client, for example one built with a custom `reqwest::Client`.
    pub fn from_async(inner: crate::Client) -> Result<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| Error::Config(format!("could not start the runtime: {e}")))?;
        Ok(Self {
            inner,
            rt: Arc::new(rt),
        })
    }

    /// A copy whose calls use `options`. See [`crate::Client::with_options`].
    pub fn with_options(&self, options: RequestOptions) -> Client {
        Client {
            inner: self.inner.with_options(options),
            rt: self.rt.clone(),
        }
    }

    /// The async client underneath.
    pub fn as_async(&self) -> &crate::Client {
        &self.inner
    }

    fn run<F: Future>(&self, f: F) -> F::Output {
        self.rt.block_on(f)
    }

    /// Uploads.
    pub fn uploads(&self) -> Uploads {
        Uploads(self.clone())
    }

    /// Dubs.
    pub fn dubs(&self) -> Dubs {
        Dubs(self.clone())
    }

    /// Requests for changes to dubs. Early access.
    pub fn runs(&self) -> Runs {
        Runs(self.clone())
    }

    /// Webhook endpoints.
    pub fn webhook_endpoints(&self) -> WebhookEndpoints {
        WebhookEndpoints(self.clone())
    }

    /// The languages dubs can be made in.
    pub fn languages(&self) -> Languages {
        Languages(self.clone())
    }

    /// Usage.
    pub fn usage(&self) -> Usage {
        Usage(self.clone())
    }

    /// See [`crate::Client::dub_file`].
    pub fn dub_file(&self, path: impl AsRef<Path>, params: DubParams) -> Result<DubsCreated> {
        self.run(self.inner.dub_file(path, params))
    }

    /// See [`crate::Client::dub_url`].
    pub fn dub_url(&self, url: impl Into<String>, params: DubParams) -> Result<DubsCreated> {
        self.run(self.inner.dub_url(url, params))
    }
}

/// Blocking [`resources::Uploads`](crate::resources::Uploads).
#[derive(Debug, Clone)]
pub struct Uploads(Client);

impl Uploads {
    /// See [`resources::Uploads::create`](crate::resources::Uploads::create).
    pub fn create(&self, params: CreateUploadParams) -> Result<Upload> {
        self.0.run(self.0.inner.uploads().create(params))
    }

    /// See [`resources::Uploads::sign_parts`](crate::resources::Uploads::sign_parts).
    pub fn sign_parts(&self, upload_id: &str, part_numbers: &[u32]) -> Result<Vec<PartLink>> {
        self.0
            .run(self.0.inner.uploads().sign_parts(upload_id, part_numbers))
    }

    /// See [`resources::Uploads::complete`](crate::resources::Uploads::complete).
    pub fn complete(&self, upload_id: &str, parts: &[UploadedPart]) -> Result<Upload> {
        self.0.run(self.0.inner.uploads().complete(upload_id, parts))
    }

    /// See [`resources::Uploads::retrieve`](crate::resources::Uploads::retrieve).
    pub fn retrieve(&self, upload_id: &str) -> Result<Upload> {
        self.0.run(self.0.inner.uploads().retrieve(upload_id))
    }

    /// See [`resources::Uploads::upload_file`](crate::resources::Uploads::upload_file).
    pub fn upload_file(&self, path: impl AsRef<Path>) -> Result<Upload> {
        self.0.run(self.0.inner.uploads().upload_file(path))
    }

    /// See [`resources::Uploads::upload_file_with`](crate::resources::Uploads::upload_file_with).
    pub fn upload_file_with(&self, path: impl AsRef<Path>, options: UploadOptions) -> Result<Upload> {
        self.0.run(self.0.inner.uploads().upload_file_with(path, options))
    }

    /// See [`resources::Uploads::upload_bytes`](crate::resources::Uploads::upload_bytes).
    pub fn upload_bytes(
        &self,
        bytes: impl Into<Bytes>,
        filename: &str,
        options: UploadOptions,
    ) -> Result<Upload> {
        self.0
            .run(self.0.inner.uploads().upload_bytes(bytes, filename, options))
    }

    /// See [`resources::Uploads::from_url`](crate::resources::Uploads::from_url).
    pub fn from_url(&self, source_url: impl Into<String>, options: FromUrlOptions) -> Result<Upload> {
        self.0.run(self.0.inner.uploads().from_url(source_url, options))
    }

    /// See [`resources::Uploads::wait_until_ready`](crate::resources::Uploads::wait_until_ready).
    pub fn wait_until_ready(&self, upload_id: &str, options: WaitOptions) -> Result<Upload> {
        self.0
            .run(self.0.inner.uploads().wait_until_ready(upload_id, options))
    }
}

/// Blocking [`resources::Dubs`](crate::resources::Dubs).
#[derive(Debug, Clone)]
pub struct Dubs(Client);

impl Dubs {
    /// Asks for changes to a finished dub. Early access.
    pub fn requests(&self) -> DubRequests {
        DubRequests(self.0.clone())
    }

    /// Reads and edits a dub's lines. Early access.
    pub fn lines(&self) -> DubLinesApi {
        DubLinesApi(self.0.clone())
    }

    /// See [`resources::Dubs::create`](crate::resources::Dubs::create).
    pub fn create(&self, params: CreateDubParams) -> Result<DubsCreated> {
        self.0.run(self.0.inner.dubs().create(params))
    }

    /// See [`resources::Dubs::retrieve`](crate::resources::Dubs::retrieve).
    pub fn retrieve(&self, dub_id: &str) -> Result<Dub> {
        self.0.run(self.0.inner.dubs().retrieve(dub_id))
    }

    /// See [`resources::Dubs::list`](crate::resources::Dubs::list).
    pub fn list(&self, params: ListDubsParams) -> Result<DubList> {
        self.0.run(self.0.inner.dubs().list(params))
    }

    /// Every dub, newest first, fetching pages as the iterator is read.
    pub fn list_all(&self, page_size: Option<u32>) -> ListAll {
        ListAll {
            client: self.0.clone(),
            limit: page_size.unwrap_or(100).clamp(1, 100),
            offset: 0,
            buffer: VecDeque::new(),
            seen: Default::default(),
            more: true,
            failed: false,
        }
    }

    /// See [`resources::Dubs::cancel`](crate::resources::Dubs::cancel).
    pub fn cancel(&self, dub_id: &str) -> Result<()> {
        self.0.run(self.0.inner.dubs().cancel(dub_id))
    }

    /// See [`resources::Dubs::files`](crate::resources::Dubs::files).
    pub fn files(&self, dub_id: &str) -> Result<DubFiles> {
        self.0.run(self.0.inner.dubs().files(dub_id))
    }

    /// See [`resources::Dubs::transcript`](crate::resources::Dubs::transcript).
    pub fn transcript(&self, dub_id: &str) -> Result<Transcript> {
        self.0.run(self.0.inner.dubs().transcript(dub_id))
    }

    /// See [`resources::Dubs::quality`](crate::resources::Dubs::quality).
    pub fn quality(&self, dub_id: &str) -> Result<Quality> {
        self.0.run(self.0.inner.dubs().quality(dub_id))
    }

    /// See [`resources::Dubs::subtitles`](crate::resources::Dubs::subtitles).
    pub fn subtitles(&self, dub_id: &str, params: SubtitlesParams) -> Result<String> {
        self.0.run(self.0.inner.dubs().subtitles(dub_id, params))
    }

    /// See [`resources::Dubs::wait_until_done`](crate::resources::Dubs::wait_until_done).
    pub fn wait_until_done(&self, dub_id: &str, options: WaitForDubOptions) -> Result<Dub> {
        self.0.run(self.0.inner.dubs().wait_until_done(dub_id, options))
    }
}

/// The iterator returned by [`Dubs::list_all`]. It ends after the first error it yields.
#[derive(Debug)]
pub struct ListAll {
    client: Client,
    limit: u32,
    offset: u32,
    buffer: VecDeque<Dub>,
    seen: std::collections::HashSet<String>,
    more: bool,
    failed: bool,
}

impl Iterator for ListAll {
    type Item = Result<Dub>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.failed {
                return None;
            }
            if let Some(dub) = self.buffer.pop_front() {
                // Dubs started while paging shift later pages by one; skip the repeats.
                if self.seen.insert(dub.id.clone()) {
                    return Some(Ok(dub));
                }
                continue;
            }
            if !self.more {
                return None;
            }
            let params = ListDubsParams::new().limit(self.limit).offset(self.offset);
            match self.client.dubs().list(params) {
                Ok(page) => {
                    self.offset += self.limit;
                    self.more = page.has_more && !page.data.is_empty();
                    self.buffer.extend(page.data);
                }
                Err(e) => {
                    self.failed = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

/// Blocking [`resources::DubRequests`](crate::resources::DubRequests).
#[derive(Debug, Clone)]
pub struct DubRequests(Client);

impl DubRequests {
    /// See [`resources::DubRequests::create`](crate::resources::DubRequests::create).
    pub fn create(&self, dub_id: &str, params: CreateRunParams) -> Result<Run> {
        self.0.run(self.0.inner.dubs().requests().create(dub_id, params))
    }
}

/// Blocking [`resources::DubLinesApi`](crate::resources::DubLinesApi).
#[derive(Debug, Clone)]
pub struct DubLinesApi(Client);

impl DubLinesApi {
    /// See [`resources::DubLinesApi::list`](crate::resources::DubLinesApi::list).
    pub fn list(&self, dub_id: &str) -> Result<crate::types::DubLines> {
        self.0.run(self.0.inner.dubs().lines().list(dub_id))
    }

    /// See [`resources::DubLinesApi::edit`](crate::resources::DubLinesApi::edit).
    pub fn edit(&self, dub_id: &str, lines: Vec<LineEdit>) -> Result<LinesEdited> {
        self.0.run(self.0.inner.dubs().lines().edit(dub_id, lines))
    }
}

/// Blocking [`resources::Runs`](crate::resources::Runs).
#[derive(Debug, Clone)]
pub struct Runs(Client);

impl Runs {
    /// See [`resources::Runs::retrieve`](crate::resources::Runs::retrieve).
    pub fn retrieve(&self, run_id: &str) -> Result<Run> {
        self.0.run(self.0.inner.runs().retrieve(run_id))
    }

    /// See [`resources::Runs::answer`](crate::resources::Runs::answer).
    pub fn answer(&self, run_id: &str, choice: RunChoice) -> Result<Run> {
        self.0.run(self.0.inner.runs().answer(run_id, choice))
    }
}

/// Blocking [`resources::WebhookEndpoints`](crate::resources::WebhookEndpoints).
#[derive(Debug, Clone)]
pub struct WebhookEndpoints(Client);

impl WebhookEndpoints {
    /// See [`resources::WebhookEndpoints::create`](crate::resources::WebhookEndpoints::create).
    pub fn create(&self, params: CreateWebhookEndpointParams) -> Result<WebhookEndpoint> {
        self.0.run(self.0.inner.webhook_endpoints().create(params))
    }

    /// See [`resources::WebhookEndpoints::list`](crate::resources::WebhookEndpoints::list).
    pub fn list(&self) -> Result<WebhookEndpointList> {
        self.0.run(self.0.inner.webhook_endpoints().list())
    }

    /// See [`resources::WebhookEndpoints::delete`](crate::resources::WebhookEndpoints::delete).
    pub fn delete(&self, endpoint_id: &str) -> Result<()> {
        self.0.run(self.0.inner.webhook_endpoints().delete(endpoint_id))
    }

    /// See [`resources::WebhookEndpoints::send_test`](crate::resources::WebhookEndpoints::send_test).
    pub fn send_test(&self, endpoint_id: &str) -> Result<()> {
        self.0
            .run(self.0.inner.webhook_endpoints().send_test(endpoint_id))
    }

    /// See [`resources::WebhookEndpoints::deliveries`](crate::resources::WebhookEndpoints::deliveries).
    pub fn deliveries(&self, endpoint_id: &str, limit: Option<u32>) -> Result<WebhookDeliveryList> {
        self.0
            .run(self.0.inner.webhook_endpoints().deliveries(endpoint_id, limit))
    }

    /// See [`resources::WebhookEndpoints::resend_delivery`](crate::resources::WebhookEndpoints::resend_delivery).
    pub fn resend_delivery(&self, endpoint_id: &str, delivery_id: &str) -> Result<WebhookDelivery> {
        self.0.run(
            self.0
                .inner
                .webhook_endpoints()
                .resend_delivery(endpoint_id, delivery_id),
        )
    }
}

/// Blocking [`resources::LanguagesApi`](crate::resources::LanguagesApi).
#[derive(Debug, Clone)]
pub struct Languages(Client);

impl Languages {
    /// See [`resources::LanguagesApi::list`](crate::resources::LanguagesApi::list).
    pub fn list(&self) -> Result<crate::types::Languages> {
        self.0.run(self.0.inner.languages().list())
    }
}

/// Blocking [`resources::UsageApi`](crate::resources::UsageApi).
#[derive(Debug, Clone)]
pub struct Usage(Client);

impl Usage {
    /// See [`resources::UsageApi::retrieve`](crate::resources::UsageApi::retrieve).
    pub fn retrieve(&self) -> Result<crate::types::Usage> {
        self.0.run(self.0.inner.usage().retrieve())
    }

    /// See [`resources::UsageApi::report`](crate::resources::UsageApi::report).
    pub fn report(&self, month: Option<&str>) -> Result<UsageReport> {
        self.0.run(self.0.inner.usage().report(month))
    }
}
