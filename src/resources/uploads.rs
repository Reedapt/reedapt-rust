use std::collections::HashMap;
use std::fmt;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::client::{backoff, Client, Req, Retry};
use crate::error::{Error, Result};
use crate::resources::seg;
use crate::types::{CreateUploadParams, PartLink, Upload, UploadStatus, UploadedPart};

/// How many links to sign in one call.
const SIGN_BATCH: u32 = 100;
const PART_ATTEMPTS: u32 = 4;

/// How far an upload has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct UploadProgress {
    /// Bytes storage has accepted so far.
    pub uploaded_bytes: u64,
    /// The file's size.
    pub total_bytes: u64,
}

/// Options for sending a file.
#[derive(Clone)]
#[non_exhaustive]
pub struct UploadOptions {
    /// Needed for readers and bytes; taken from the path otherwise. Keep the extension.
    pub filename: Option<String>,
    /// Parts sent at once. Defaults to 4. Each is held in memory while it is sent (the API
    /// chooses the part size, 16 MiB or more).
    pub concurrency: usize,
    /// Called after each part is accepted.
    pub on_progress: Option<Arc<dyn Fn(UploadProgress) + Send + Sync>>,
}

impl Default for UploadOptions {
    fn default() -> Self {
        Self {
            filename: None,
            concurrency: 4,
            on_progress: None,
        }
    }
}

impl UploadOptions {
    /// The defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// The file's name, with its extension.
    pub fn filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// How many parts to send at once.
    pub fn concurrency(mut self, concurrency: usize) -> Self {
        self.concurrency = concurrency.max(1);
        self
    }

    /// Calls `f` after each part is accepted.
    pub fn on_progress(mut self, f: impl Fn(UploadProgress) + Send + Sync + 'static) -> Self {
        self.on_progress = Some(Arc::new(f));
        self
    }
}

impl fmt::Debug for UploadOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UploadOptions")
            .field("filename", &self.filename)
            .field("concurrency", &self.concurrency)
            .field("on_progress", &self.on_progress.is_some())
            .finish()
    }
}

/// How often to check and how long to keep checking.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct WaitOptions {
    /// Time between checks.
    pub poll_interval: Option<Duration>,
    /// How long before giving up.
    pub timeout: Option<Duration>,
}

impl WaitOptions {
    /// The defaults for the thing being waited on.
    pub fn new() -> Self {
        Self::default()
    }

    /// Time between checks.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = Some(interval);
        self
    }

    /// How long before giving up.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

/// Options for [`Uploads::from_url`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct FromUrlOptions {
    /// Only when the link doesn't end in a file name. Keep the extension.
    pub filename: Option<String>,
    /// Return as soon as the upload is created instead of waiting until the download is
    /// done.
    pub no_wait: bool,
    /// How often to check and for how long, when waiting.
    pub wait: WaitOptions,
}

impl FromUrlOptions {
    /// Wait until the download is done.
    pub fn new() -> Self {
        Self::default()
    }

    /// The file's name, with its extension.
    pub fn filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Return without waiting for the download.
    pub fn no_wait(mut self) -> Self {
        self.no_wait = true;
        self
    }
}

/// Getting files to Reedapt.
#[derive(Debug, Clone)]
pub struct Uploads {
    client: Client,
}

impl Uploads {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    /// Starts an upload. Most code wants [`upload_file`](Self::upload_file) or
    /// [`from_url`](Self::from_url) instead. Always sent with an `Idempotency-Key`.
    pub async fn create(&self, params: CreateUploadParams) -> Result<Upload> {
        self.client
            .json(Req::post("/uploads", Retry::Safe).json(&params)?.idempotent())
            .await
    }

    /// Signs links for up to 100 more parts.
    pub async fn sign_parts(&self, upload_id: &str, part_numbers: &[u32]) -> Result<Vec<PartLink>> {
        #[derive(Serialize)]
        struct Body<'a> {
            part_numbers: &'a [u32],
        }
        #[derive(Deserialize)]
        struct Answer {
            parts: Vec<PartLink>,
        }
        let answer: Answer = self
            .client
            .json(
                Req::post(format!("/uploads/{}/parts", seg(upload_id)), Retry::Safe)
                    .json(&Body { part_numbers })?,
            )
            .await?;
        Ok(answer.parts)
    }

    /// Finishes an upload once every part is sent.
    pub async fn complete(&self, upload_id: &str, parts: &[UploadedPart]) -> Result<Upload> {
        #[derive(Serialize)]
        struct Body<'a> {
            parts: &'a [UploadedPart],
        }
        self.client
            .json(
                Req::post(format!("/uploads/{}/complete", seg(upload_id)), Retry::Safe)
                    .json(&Body { parts })?,
            )
            .await
    }

    /// One upload.
    pub async fn retrieve(&self, upload_id: &str) -> Result<Upload> {
        self.client
            .json(Req::get(format!("/uploads/{}", seg(upload_id))))
            .await
    }

    /// Sends a file from disk straight to storage in parts, four at a time, and completes
    /// the upload. Expired links are signed again and failed parts are retried.
    pub async fn upload_file(&self, path: impl AsRef<Path>) -> Result<Upload> {
        self.upload_file_with(path, UploadOptions::default()).await
    }

    /// [`upload_file`](Self::upload_file) with options.
    pub async fn upload_file_with(&self, path: impl AsRef<Path>, options: UploadOptions) -> Result<Upload> {
        let path = path.as_ref();
        let file = tokio::fs::File::open(path).await?;
        let size = file.metadata().await?.len();
        let filename = options
            .filename
            .clone()
            .or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()))
            .ok_or_else(|| {
                Error::Upload(format!(
                    "{} has no file name; set one in the options.",
                    path.display()
                ))
            })?;
        self.upload_reader(file, &filename, size, options).await
    }

    /// Sends bytes already in memory.
    pub async fn upload_bytes(
        &self,
        bytes: impl Into<Bytes>,
        filename: &str,
        options: UploadOptions,
    ) -> Result<Upload> {
        let bytes: Bytes = bytes.into();
        let size = bytes.len() as u64;
        self.upload_reader(std::io::Cursor::new(bytes), filename, size, options)
            .await
    }

    /// Sends anything that can be read asynchronously. `size` must be the exact number of
    /// bytes the reader will give, because the API divides the file into parts up front.
    /// Only the parts being sent are held in memory.
    pub async fn upload_reader<R>(
        &self,
        mut reader: R,
        filename: &str,
        size: u64,
        options: UploadOptions,
    ) -> Result<Upload>
    where
        R: AsyncRead + Unpin,
    {
        if size == 0 {
            return Err(Error::Upload("the file is empty".into()));
        }
        let upload = self.create(CreateUploadParams::file(filename, size)).await?;
        // Only the first call carries the caller's idempotency key.
        let client = self.client.with_idempotency_key(None);
        let uploads = Uploads::new(client.clone());

        let mut links: HashMap<u32, String> = upload
            .parts
            .iter()
            .map(|p| (p.part_number, p.url.clone()))
            .collect();
        let concurrency = options.concurrency.max(1);
        let mut in_flight = FuturesUnordered::new();
        let mut done: Vec<UploadedPart> = Vec::with_capacity(upload.part_count as usize);
        let mut uploaded_bytes = 0u64;
        let mut next = 1u32;

        loop {
            while next <= upload.part_count && in_flight.len() < concurrency {
                let n = next;
                next += 1;
                let start = u64::from(n - 1) * upload.part_size;
                let len = upload.part_size.min(size.saturating_sub(start));
                let mut buf = vec![0u8; len as usize];
                reader.read_exact(&mut buf).await.map_err(|e| {
                    if e.kind() == ErrorKind::UnexpectedEof {
                        Error::Upload(format!(
                            "the file ended before part {n}; its size changed or {size} was wrong"
                        ))
                    } else {
                        Error::Io(e)
                    }
                })?;
                if !links.contains_key(&n) {
                    let last = (n + SIGN_BATCH - 1).min(upload.part_count);
                    let wanted: Vec<u32> = (n..=last).filter(|p| !links.contains_key(p)).collect();
                    for link in uploads.sign_parts(&upload.id, &wanted).await? {
                        links.insert(link.part_number, link.url);
                    }
                }
                let url = links.get(&n).cloned().ok_or_else(|| {
                    Error::Upload(format!("No link was signed for part {n} of {}.", upload.id))
                })?;
                in_flight.push(put_part(
                    client.clone(),
                    upload.id.clone(),
                    n,
                    Bytes::from(buf),
                    url,
                ));
            }
            match in_flight.next().await {
                Some(part) => {
                    let (part, bytes) = part?;
                    done.push(part);
                    uploaded_bytes += bytes;
                    if let Some(f) = &options.on_progress {
                        f(UploadProgress {
                            uploaded_bytes,
                            total_bytes: size,
                        });
                    }
                }
                None => break,
            }
        }

        done.sort_by_key(|p| p.part_number);
        match uploads.complete(&upload.id, &done).await {
            // A retried complete that had already gone through.
            Err(Error::Api {
                status: 409, code, ..
            }) if code == "UPLOAD_COMPLETE" => uploads.retrieve(&upload.id).await,
            other => other,
        }
    }

    /// Has Reedapt download a public link. Waits until it's ready unless
    /// [`FromUrlOptions::no_wait`] is set.
    pub async fn from_url(&self, source_url: impl Into<String>, options: FromUrlOptions) -> Result<Upload> {
        let mut params = CreateUploadParams::url(source_url);
        params.filename = options.filename;
        let upload = self.create(params).await?;
        if options.no_wait {
            return Ok(upload);
        }
        Uploads::new(self.client.with_idempotency_key(None))
            .wait_until_ready(&upload.id, options.wait)
            .await
    }

    /// Polls an upload until it can be dubbed. Returns [`Error::Wait`] if it failed, was
    /// already dubbed, or is still going when the timeout (3 hours) runs out.
    pub async fn wait_until_ready(&self, upload_id: &str, options: WaitOptions) -> Result<Upload> {
        let interval = options.poll_interval.unwrap_or(Duration::from_secs(10));
        let deadline = Instant::now() + options.timeout.unwrap_or(Duration::from_secs(3 * 60 * 60));
        loop {
            let upload = self.retrieve(upload_id).await?;
            match upload.status {
                UploadStatus::Uploaded => return Ok(upload),
                UploadStatus::Failed => {
                    return Err(Error::Wait(format!(
                        "Upload {upload_id} failed: {}.",
                        upload.error.as_deref().unwrap_or("no reason given")
                    )))
                }
                UploadStatus::Used => {
                    return Err(Error::Wait(format!(
                        "Upload {upload_id} was already dubbed. Upload the file again."
                    )))
                }
                _ => {}
            }
            if Instant::now() + interval > deadline {
                return Err(Error::Wait(format!(
                    "Upload {upload_id} was still {:?} when the wait ran out.",
                    upload.status
                )));
            }
            tokio::time::sleep(interval).await;
        }
    }
}

/// Sends one part to storage. No API key goes with it: the signed link carries its own
/// permission. Expired or refused links are signed again.
async fn put_part(
    client: Client,
    upload_id: String,
    n: u32,
    bytes: Bytes,
    first_url: String,
) -> Result<(UploadedPart, u64)> {
    let mut url = first_url;
    let mut last_problem = String::new();
    for attempt in 0..PART_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(backoff(attempt)).await;
        }
        let sent = client
            .http()
            .put(&url)
            .timeout(client.timeout())
            .body(bytes.clone())
            .send()
            .await;
        let res = match sent {
            Ok(res) => res,
            Err(e) => {
                last_problem = e.to_string();
                continue;
            }
        };
        let status = res.status();
        if status.is_success() {
            let etag = res
                .headers()
                .get(reqwest::header::ETAG)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
                .ok_or_else(|| Error::Upload(format!("Storage accepted part {n} but sent no ETag.")))?;
            return Ok((UploadedPart::new(n, etag), bytes.len() as u64));
        }
        last_problem = format!("storage answered {}", status.as_u16());
        match status.as_u16() {
            // An expired or refused link: sign this part again.
            400 | 403 => {
                if let Some(link) = Uploads::new(client.clone())
                    .sign_parts(&upload_id, &[n])
                    .await?
                    .into_iter()
                    .find(|l| l.part_number == n)
                {
                    url = link.url;
                }
            }
            408 | 429 | 500..=599 => {}
            _ => break,
        }
    }
    Err(Error::Upload(format!(
        "Part {n} of upload {upload_id} failed: {last_problem}."
    )))
}
