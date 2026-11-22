use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::stream::{self, Stream};
use serde::Serialize;

use crate::client::{Client, Req, Retry};
use crate::error::{Error, Result};
use crate::resources::seg;
use crate::resources::uploads::WaitOptions;
use crate::types::{
    CreateDubParams, CreateRunParams, Dub, DubFiles, DubLines, DubList, DubsCreated, LineEdit, LinesEdited,
    ListDubsParams, Quality, Run, RunChoice, SubtitlesParams, Transcript,
};

type OnUpdate = Arc<dyn Fn(&Dub) + Send + Sync>;

/// Options for [`Dubs::wait_until_done`].
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct WaitForDubOptions {
    /// How often to check, and how long to keep checking.
    pub wait: WaitOptions,
    /// Called after each check, for example to show progress.
    pub on_update: Option<OnUpdate>,
}

impl WaitForDubOptions {
    /// Checks every 30 seconds for up to 12 hours.
    pub fn new() -> Self {
        Self::default()
    }

    /// How long to wait between checks.
    pub fn poll_interval(mut self, interval: Duration) -> Self {
        self.wait.poll_interval = Some(interval);
        self
    }

    /// How long to keep checking before giving up.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.wait.timeout = Some(timeout);
        self
    }

    /// Calls `f` with the dub after each check.
    pub fn on_update(mut self, f: impl Fn(&Dub) + Send + Sync + 'static) -> Self {
        self.on_update = Some(Arc::new(f));
        self
    }
}

impl fmt::Debug for WaitForDubOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WaitForDubOptions")
            .field("wait", &self.wait)
            .field("on_update", &self.on_update.is_some())
            .finish()
    }
}

/// Dubs: one video or audio file into one language each.
#[derive(Debug, Clone)]
pub struct Dubs {
    client: Client,
}

impl Dubs {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    /// Asks for changes to a finished dub. Early access.
    pub fn requests(&self) -> DubRequests {
        DubRequests {
            client: self.client.clone(),
        }
    }

    /// Reads and edits a dub's lines. Early access.
    pub fn lines(&self) -> DubLinesApi {
        DubLinesApi {
            client: self.client.clone(),
        }
    }

    /// Starts one dub per target language. Languages that can't start come back in
    /// [`DubsCreated::refused`].
    ///
    /// Always sent with an `Idempotency-Key`, so a retry never makes a second dub. Set your
    /// own with [`Client::with_options`] to make a repeat safe across restarts.
    pub async fn create(&self, params: CreateDubParams) -> Result<DubsCreated> {
        self.client
            .json(Req::post("/dubs", Retry::Safe).json(&params)?.idempotent())
            .await
    }

    /// One dub.
    pub async fn retrieve(&self, dub_id: &str) -> Result<Dub> {
        self.client.json(Req::get(format!("/dubs/{}", seg(dub_id)))).await
    }

    /// One page of the account's dubs, newest first.
    pub async fn list(&self, params: ListDubsParams) -> Result<DubList> {
        let mut req = Req::get("/dubs");
        if let Some(limit) = params.limit {
            req = req.query("limit", limit);
        }
        if let Some(offset) = params.offset {
            req = req.query("offset", offset);
        }
        self.client.json(req).await
    }

    /// Every dub, newest first, fetching pages as the stream is read. `page_size` is 1 to
    /// 100 and defaults to 100 when `None`.
    ///
    /// ```no_run
    /// use futures_util::TryStreamExt;
    ///
    /// # async fn run(client: reedapt::Client) -> reedapt::Result<()> {
    /// let mut dubs = std::pin::pin!(client.dubs().list_all(None));
    /// while let Some(dub) = dubs.try_next().await? {
    ///     println!("{} {:?}", dub.id, dub.status);
    /// }
    /// # Ok(()) }
    /// ```
    ///
    /// The stream ends after the first error it yields.
    pub fn list_all(&self, page_size: Option<u32>) -> impl Stream<Item = Result<Dub>> + Send + 'static {
        struct State {
            dubs: Dubs,
            limit: u32,
            offset: u32,
            buffer: VecDeque<Dub>,
            seen: HashSet<String>,
            more: bool,
            failed: bool,
        }
        let state = State {
            dubs: self.clone(),
            limit: page_size.unwrap_or(100).clamp(1, 100),
            offset: 0,
            buffer: VecDeque::new(),
            seen: HashSet::new(),
            more: true,
            failed: false,
        };
        stream::unfold(state, |mut s| async move {
            loop {
                if s.failed {
                    return None;
                }
                if let Some(dub) = s.buffer.pop_front() {
                    // Dubs started while paging shift later pages by one; skip the repeats.
                    if s.seen.insert(dub.id.clone()) {
                        return Some((Ok(dub), s));
                    }
                    continue;
                }
                if !s.more {
                    return None;
                }
                let params = ListDubsParams::new().limit(s.limit).offset(s.offset);
                match s.dubs.list(params).await {
                    Ok(page) => {
                        s.offset += s.limit;
                        s.more = page.has_more && !page.data.is_empty();
                        s.buffer.extend(page.data);
                    }
                    Err(e) => {
                        s.failed = true;
                        return Some((Err(e), s));
                    }
                }
            }
        })
    }

    /// Stops an unfinished dub and deletes it. Its minutes are returned.
    pub async fn cancel(&self, dub_id: &str) -> Result<()> {
        self.client
            .empty(Req::post(format!("/dubs/{}/cancel", seg(dub_id)), Retry::Safe))
            .await
    }

    /// Download links for a completed dub's files. Each link lasts an hour.
    pub async fn files(&self, dub_id: &str) -> Result<DubFiles> {
        self.client
            .json(Req::get(format!("/dubs/{}/files", seg(dub_id))))
            .await
    }

    /// Every line of a completed dub with its timing, speaker, original words and
    /// translation.
    pub async fn transcript(&self, dub_id: &str) -> Result<Transcript> {
        self.client
            .json(Req::get(format!("/dubs/{}/transcript", seg(dub_id))))
            .await
    }

    /// Reedapt's check of a finished dub: what it listened to, what it fixed and the lines
    /// left for you to review. [`Quality::checked`] is false until the check has run; the
    /// `dub.checked` webhook announces it.
    pub async fn quality(&self, dub_id: &str) -> Result<Quality> {
        self.client
            .json(Req::get(format!("/dubs/{}/quality", seg(dub_id))))
            .await
    }

    /// A completed dub's subtitles as SRT or WebVTT text.
    pub async fn subtitles(&self, dub_id: &str, params: SubtitlesParams) -> Result<String> {
        let mut req = Req::get(format!("/dubs/{}/subtitles", seg(dub_id)));
        if let Some(format) = params.format {
            req = req.query("format", enum_name(&format));
        }
        if let Some(language) = params.language {
            req = req.query("language", enum_name(&language));
        }
        self.client.text(req).await
    }

    /// Polls a dub until it is completed or failed and returns it. Check
    /// [`Dub::status`] on the result. Webhooks are better for anything long-running.
    ///
    /// Returns [`Error::Wait`] if the dub is still going when the timeout (12 hours by
    /// default) runs out.
    pub async fn wait_until_done(&self, dub_id: &str, options: WaitForDubOptions) -> Result<Dub> {
        let interval = options.wait.poll_interval.unwrap_or(Duration::from_secs(30));
        let deadline = Instant::now() + options.wait.timeout.unwrap_or(Duration::from_secs(12 * 60 * 60));
        loop {
            let dub = self.retrieve(dub_id).await?;
            if let Some(f) = &options.on_update {
                f(&dub);
            }
            if dub.status.is_finished() {
                return Ok(dub);
            }
            if Instant::now() + interval > deadline {
                return Err(Error::Wait(format!(
                    "Dub {dub_id} was still {:?} when the wait ran out.",
                    dub.status
                )));
            }
            tokio::time::sleep(interval).await;
        }
    }
}

fn enum_name(value: &impl Serialize) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

/// Asking Reedapt for changes to a finished dub. Early access: other accounts get a 403
/// with the code `NOT_AVAILABLE`.
#[derive(Debug, Clone)]
pub struct DubRequests {
    client: Client,
}

impl DubRequests {
    /// Hands Reedapt a request in plain words, such as "Make line 3 sound calmer". With
    /// the default [`Autonomy::Ask`](crate::types::Autonomy::Ask) nothing changes until you
    /// answer the run with [`Runs::answer`].
    ///
    /// Not retried after a network failure: it could start the request twice.
    pub async fn create(&self, dub_id: &str, params: CreateRunParams) -> Result<Run> {
        self.client
            .json(Req::post(format!("/dubs/{}/requests", seg(dub_id)), Retry::Unsafe).json(&params)?)
            .await
    }
}

/// Reading and editing a finished dub's lines. Early access.
#[derive(Debug, Clone)]
pub struct DubLinesApi {
    client: Client,
}

impl DubLinesApi {
    /// Every line with its timing, speaker, original and dubbed words, and delivery notes.
    pub async fn list(&self, dub_id: &str) -> Result<DubLines> {
        self.client
            .json(Req::get(format!("/dubs/{}/lines", seg(dub_id))))
            .await
    }

    /// Changes up to 100 lines. They are voiced again in the background; the dub's files
    /// update when that finishes.
    pub async fn edit(&self, dub_id: &str, lines: Vec<LineEdit>) -> Result<LinesEdited> {
        #[derive(Serialize)]
        struct Body {
            lines: Vec<LineEdit>,
        }
        self.client
            .json(
                Req::post(format!("/dubs/{}/lines/edit", seg(dub_id)), Retry::Unsafe)
                    .json(&Body { lines })?,
            )
            .await
    }
}

/// Requests for changes to dubs. Early access.
#[derive(Debug, Clone)]
pub struct Runs {
    client: Client,
}

impl Runs {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    /// One run.
    pub async fn retrieve(&self, run_id: &str) -> Result<Run> {
        self.client.json(Req::get(format!("/runs/{}", seg(run_id)))).await
    }

    /// Answers a run that is `needs_input`: apply the proposal or decline it.
    pub async fn answer(&self, run_id: &str, choice: RunChoice) -> Result<Run> {
        #[derive(Serialize)]
        struct Body {
            choice: RunChoice,
        }
        self.client
            .json(Req::post(format!("/runs/{}/answer", seg(run_id)), Retry::Unsafe).json(&Body { choice })?)
            .await
    }
}
