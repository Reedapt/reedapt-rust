//! The objects the API returns and the parameters it takes.
//!
//! The API may add fields and enum values over time. Response structs and enums are
//! `#[non_exhaustive]`, and enums have an `Unknown` variant that catches values this
//! version doesn't know, so an older crate keeps working.
//!
//! Parameter structs are built with `new` and chained setters:
//!
//! ```
//! use reedapt::types::CreateDubParams;
//!
//! let params = CreateDubParams::new("upl_123", "en", ["fr", "es"])
//!     .voice_cloning(true)
//!     .metadata("order_id", "1042");
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------- uploads

/// Where an upload is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UploadStatus {
    /// Parts are being sent.
    Uploading,
    /// Reedapt is downloading a link.
    Fetching,
    /// Ready to be dubbed.
    Uploaded,
    /// Already dubbed. Upload the file again to dub it again.
    Used,
    /// Something went wrong; see [`Upload::error`].
    Failed,
    /// A status this version of the crate doesn't know.
    #[serde(other)]
    Unknown,
}

/// A signed link for one part of an upload.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct PartLink {
    /// Starts at 1.
    pub part_number: u32,
    /// `PUT` the part's bytes here. The link carries its own permission.
    pub url: String,
}

/// A file that has been, or is being, handed to Reedapt.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Upload {
    /// The id to pass as `upload_id` when creating dubs.
    pub id: String,
    /// The file's name.
    pub filename: String,
    /// Its size in bytes.
    pub size: u64,
    /// Where it is in its life.
    pub status: UploadStatus,
    /// Why it failed, when it did.
    #[serde(default)]
    pub error: Option<String>,
    /// How big each part is, in bytes (the last may be smaller).
    pub part_size: u64,
    /// How many parts there are.
    pub part_count: u32,
    /// Signed links for the first parts.
    #[serde(default)]
    pub parts: Vec<PartLink>,
    /// When the links stop working.
    #[serde(default)]
    pub parts_expire_at: Option<String>,
}

/// A part that has been sent, for completing an upload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UploadedPart {
    /// Starts at 1.
    pub part_number: u32,
    /// The `ETag` storage answered with.
    pub etag: String,
}

impl UploadedPart {
    /// A part number with the `ETag` storage answered with.
    pub fn new(part_number: u32, etag: impl Into<String>) -> Self {
        Self {
            part_number,
            etag: etag.into(),
        }
    }
}

/// Parameters for [`Uploads::create`](crate::resources::Uploads::create). Most code wants
/// `upload_file` or `from_url` instead.
#[derive(Debug, Clone, Default, Serialize)]
#[non_exhaustive]
pub struct CreateUploadParams {
    /// Keep the extension.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Bytes, for a file you will send in parts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// A public link Reedapt downloads itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

impl CreateUploadParams {
    /// A file you will send in parts.
    pub fn file(filename: impl Into<String>, size: u64) -> Self {
        Self {
            filename: Some(filename.into()),
            size: Some(size),
            source_url: None,
        }
    }

    /// A public link Reedapt downloads itself.
    pub fn url(source_url: impl Into<String>) -> Self {
        Self {
            filename: None,
            size: None,
            source_url: Some(source_url.into()),
        }
    }

    /// The file's name, when the link doesn't end in one.
    pub fn filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }
}

// ------------------------------------------------------------------- dubs

/// Where a dub is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DubStatus {
    /// Waiting for a free slot.
    Queued,
    /// Being dubbed; see [`Dub::stage`] and [`Dub::progress`].
    Processing,
    /// Done. Its files are available.
    Completed,
    /// Failed; see [`Dub::error`]. The minutes are returned.
    Failed,
    /// A status this version of the crate doesn't know.
    #[serde(other)]
    Unknown,
}

impl DubStatus {
    /// True once the dub has stopped changing: completed or failed.
    pub fn is_finished(self) -> bool {
        matches!(self, DubStatus::Completed | DubStatus::Failed)
    }
}

/// Why a dub failed.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct DubError {
    /// A stable code.
    pub code: String,
    /// A plain-English explanation.
    pub message: String,
}

/// One dub: one video or audio file into one language.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Dub {
    /// The dub's id.
    pub id: String,
    /// Where it is in its life.
    pub status: DubStatus,
    /// What it is doing now, while processing.
    #[serde(default)]
    pub stage: Option<String>,
    /// 0 to 100.
    pub progress: u32,
    /// The language spoken in the original.
    pub source_language: Option<String>,
    /// The language of the dub.
    pub target_language: Option<String>,
    /// Whether the file is audio only.
    pub audio: bool,
    /// Made with a test key: watermarked and limited to a preview.
    pub test: bool,
    /// How many seconds a test dub covers.
    #[serde(default)]
    pub preview_seconds: Option<u32>,
    /// The length of the media in minutes.
    #[serde(default)]
    pub duration_minutes: Option<f64>,
    /// When the dub was made (RFC 3339).
    pub created_at: String,
    /// When it started processing.
    #[serde(default)]
    pub started_at: Option<String>,
    /// When it finished.
    #[serde(default)]
    pub completed_at: Option<String>,
    /// Why it failed.
    #[serde(default)]
    pub error: Option<DubError>,
    /// The metadata you attached.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

/// A target language that could not be started.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct RefusedLanguage {
    /// The language.
    pub target_language: String,
    /// A stable code saying why.
    pub code: String,
    /// A plain-English explanation.
    pub message: String,
}

/// The answer to creating dubs.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct DubsCreated {
    /// One dub per language that started.
    pub dubs: Vec<Dub>,
    /// Languages that couldn't start, and why.
    #[serde(default)]
    pub refused: Vec<RefusedLanguage>,
}

/// One page of dubs.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct DubList {
    /// The dubs on this page, newest first.
    pub data: Vec<Dub>,
    /// Whether another page follows.
    pub has_more: bool,
}

/// The options for dubbing a file, without the upload. Used by
/// [`Client::dub_file`](crate::Client::dub_file) and, with an upload id, by
/// [`CreateDubParams`].
#[derive(Debug, Clone, Default, Serialize)]
#[non_exhaustive]
pub struct DubParams {
    /// The language spoken in the original.
    pub source_language: String,
    /// One dub is made for each.
    pub target_languages: Vec<String>,
    /// Keep each speaker's own voice. The API's default applies when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_cloning: Option<bool>,
    /// How many people speak, 1 to 20. Reedapt counts them when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_speakers: Option<u32>,
    /// What the media is, for example `interview`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_category: Option<String>,
    /// Notes for the dub, up to 2000 characters: names to keep, tone, how to address the
    /// audience. Followed on paid plans.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Your own labels, returned on the dub.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

macro_rules! dub_setters {
    ($($field:ident: $ty:ty),* $(,)?) => {
        $(
            #[doc = concat!("Sets `", stringify!($field), "`.")]
            pub fn $field(mut self, value: impl Into<$ty>) -> Self {
                self.options_mut().$field = Some(value.into());
                self
            }
        )*
        /// Attaches a metadata label, returned on the dub.
        pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
            self.options_mut().metadata.insert(key.into(), value.into());
            self
        }
    };
}

impl DubParams {
    /// Dub from `source_language` into each of `target_languages`.
    pub fn new<I, S>(source_language: impl Into<String>, target_languages: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            source_language: source_language.into(),
            target_languages: target_languages.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    fn options_mut(&mut self) -> &mut Self {
        self
    }

    dub_setters!(voice_cloning: bool, num_speakers: u32, media_category: String, instructions: String);
}

/// Parameters for [`Dubs::create`](crate::resources::Dubs::create).
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct CreateDubParams {
    /// The upload to dub.
    pub upload_id: String,
    /// The language, voice and other options.
    #[serde(flatten)]
    pub options: DubParams,
}

impl CreateDubParams {
    /// Dub the upload from `source_language` into each of `target_languages`.
    pub fn new<I, S>(
        upload_id: impl Into<String>,
        source_language: impl Into<String>,
        target_languages: I,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            upload_id: upload_id.into(),
            options: DubParams::new(source_language, target_languages),
        }
    }

    /// An upload with options already prepared.
    pub fn with_options(upload_id: impl Into<String>, options: DubParams) -> Self {
        Self {
            upload_id: upload_id.into(),
            options,
        }
    }

    fn options_mut(&mut self) -> &mut DubParams {
        &mut self.options
    }

    dub_setters!(voice_cloning: bool, num_speakers: u32, media_category: String, instructions: String);
}

/// Parameters for [`Dubs::list`](crate::resources::Dubs::list).
#[derive(Debug, Clone, Default, Serialize)]
#[non_exhaustive]
pub struct ListDubsParams {
    /// How many to return, 1 to 100. The API's default is 20.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// How many to skip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

impl ListDubsParams {
    /// The API's defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many to return, 1 to 100.
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// How many to skip.
    pub fn offset(mut self, offset: u32) -> Self {
        self.offset = Some(offset);
        self
    }
}

/// Which file a [`FileLink`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FileKind {
    /// The dubbed video or audio.
    Dub,
    /// The dubbed speech as a WAV.
    DubbedAudioWav,
    /// The voice track alone.
    Voice,
    /// The background track alone.
    Background,
    /// A kind this version of the crate doesn't know.
    #[serde(other)]
    Unknown,
}

/// A download link for one file of a dub.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct FileLink {
    /// Which file it is.
    pub kind: FileKind,
    /// Its name.
    pub filename: String,
    /// Where to download it.
    pub url: String,
}

/// Download links for a completed dub.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct DubFiles {
    /// One link per file.
    pub files: Vec<FileLink>,
    /// When the links stop working (they last an hour).
    pub expires_at: String,
}

/// One line of a transcript.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct TranscriptLine {
    /// Where the line starts, in milliseconds.
    pub start_ms: u64,
    /// Where it ends.
    pub end_ms: u64,
    /// The same label throughout a dub, such as `SPEAKER_00`.
    #[serde(default)]
    pub speaker: Option<String>,
    /// The original words.
    pub original_text: String,
    /// The line in the dub's language.
    pub text: String,
}

/// A completed dub's lines in time order, with edits made in the editor included.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Transcript {
    /// The dub.
    pub dub_id: String,
    /// The original language.
    #[serde(default)]
    pub source_language: Option<String>,
    /// The dub's language.
    #[serde(default)]
    pub target_language: Option<String>,
    /// The lines.
    pub lines: Vec<TranscriptLine>,
}

/// The format of [`Dubs::subtitles`](crate::resources::Dubs::subtitles).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SubtitleFormat {
    /// SubRip. The default.
    #[default]
    Srt,
    /// WebVTT.
    Vtt,
}

/// Which words [`Dubs::subtitles`](crate::resources::Dubs::subtitles) returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum SubtitleLanguage {
    /// The dub's language. The default.
    #[default]
    Target,
    /// The original words.
    Source,
}

/// Parameters for [`Dubs::subtitles`](crate::resources::Dubs::subtitles).
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[non_exhaustive]
pub struct SubtitlesParams {
    /// SRT (the default) or WebVTT.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<SubtitleFormat>,
    /// The dub's language (the default) or the original words.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<SubtitleLanguage>,
}

impl SubtitlesParams {
    /// The API's defaults: SRT, in the dub's language.
    pub fn new() -> Self {
        Self::default()
    }

    /// SRT or WebVTT.
    pub fn format(mut self, format: SubtitleFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// The dub's language or the original words.
    pub fn language(mut self, language: SubtitleLanguage) -> Self {
        self.language = Some(language);
        self
    }
}

// ---------------------------------------------------------------- quality

/// How a line Reedapt flagged was dealt with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QualityFix {
    /// The wording was shortened to fit.
    Shortened,
    /// The line was voiced again.
    Revoiced,
    /// A fix this version of the crate doesn't know.
    #[serde(other)]
    Unknown,
}

/// A line Reedapt's check flagged.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct FlaggedLine {
    /// The line's number, starting at 1.
    pub line: u32,
    /// Where it starts, in milliseconds.
    pub start_ms: u64,
    /// What the checks found, such as `fit` (the line doesn't fit its time).
    pub checks: Vec<String>,
    /// How it was dealt with, if it was.
    #[serde(default)]
    pub fixed: Option<QualityFix>,
}

/// Reedapt's check of a finished dub.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Quality {
    /// The dub.
    pub dub_id: String,
    /// Whether the measurement checks have run.
    pub checked: bool,
    /// Whether Reedapt listened to the dub.
    pub listened: bool,
    /// How many lines the dub has.
    pub lines: u32,
    /// How many were fixed.
    pub fixed: u32,
    /// How many are left for you to review.
    pub to_review: u32,
    /// The lines that were flagged.
    pub flagged: Vec<FlaggedLine>,
}

// ------------------------------------------------- requests, runs and lines

/// Where a [`Run`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunStatus {
    /// Answered; nothing to change.
    Done,
    /// Waiting for your answer to a proposal.
    NeedsInput,
    /// The change was made.
    Applied,
    /// You declined the proposal.
    Declined,
    /// The run failed.
    Failed,
    /// A status this version of the crate doesn't know.
    #[serde(other)]
    Unknown,
}

/// Your answer to a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RunChoice {
    /// Make the change.
    Apply,
    /// Leave the dub as it is.
    Decline,
    /// A choice this version of the crate doesn't know.
    #[serde(other, skip_serializing)]
    Unknown,
}

/// Whether Reedapt waits for your answer before changing anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Autonomy {
    /// Wait for your answer. The default.
    #[default]
    Ask,
    /// Reedapt applies what it judges right.
    Decide,
}

/// What Reedapt proposes to change.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct RunProposal {
    /// What kind of change.
    pub kind: String,
    /// In words.
    pub summary: String,
    /// How many lines it would change.
    pub lines: u32,
}

/// An answer you can give to a proposal.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct RunOption {
    /// Pass this to [`Runs::answer`](crate::resources::Runs::answer).
    pub id: RunChoice,
    /// What to show a person.
    pub label: String,
}

/// A request for changes to a dub, and what Reedapt did or proposes to do about it.
/// Early access.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Run {
    /// The run's id.
    pub id: String,
    /// The dub it is about.
    pub dub_id: String,
    /// Where it is.
    pub status: RunStatus,
    /// What you asked.
    pub request: String,
    /// Reedapt's answer.
    pub reply: String,
    /// Present while `status` is [`RunStatus::NeedsInput`].
    #[serde(default)]
    pub proposal: Option<RunProposal>,
    /// The answers you can give.
    #[serde(default)]
    pub options: Option<Vec<RunOption>>,
    /// How many lines were changed, once applied. They are voiced again in the background.
    #[serde(default)]
    pub applied_lines: Option<u32>,
    /// When the run was made.
    pub created_at: String,
}

/// Parameters for [`DubRequests::create`](crate::resources::DubRequests::create).
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct CreateRunParams {
    /// The request in plain words, such as "Make line 3 sound calmer".
    pub text: String,
    /// Whether Reedapt waits for your answer. Defaults to [`Autonomy::Ask`].
    pub autonomy: Autonomy,
}

impl CreateRunParams {
    /// Asks, and waits for your answer before changing anything.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            autonomy: Autonomy::Ask,
        }
    }

    /// Whether Reedapt waits for your answer.
    pub fn autonomy(mut self, autonomy: Autonomy) -> Self {
        self.autonomy = autonomy;
        self
    }
}

/// One line of a dub, as [`DubLines`] lists it.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct DubLine {
    /// The line's number in time order, starting at 1.
    pub line: u32,
    /// Where it starts, in milliseconds.
    pub start_ms: u64,
    /// Where it ends.
    pub end_ms: u64,
    /// The speaker's label.
    #[serde(default)]
    pub speaker: Option<String>,
    /// The original words.
    pub original_text: String,
    /// The line in the dub's language.
    pub text: String,
    /// For example `angry` or `whisper`.
    #[serde(default)]
    pub delivery_style: Option<String>,
    /// How to say it, in plain words.
    #[serde(default)]
    pub voice_instructions: Option<String>,
}

/// Every line of a dub.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct DubLines {
    /// The dub.
    pub dub_id: String,
    /// The lines in time order.
    pub lines: Vec<DubLine>,
}

/// A change to one line. Leave a field `None` to keep it as it is.
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct LineEdit {
    /// The line's number.
    pub line: u32,
    /// New wording, in the dub's language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// `Some(None)` clears it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_style: Option<Option<String>>,
    /// How to say it, in plain words, up to 300 characters. `Some(None)` clears it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_instructions: Option<Option<String>>,
}

impl LineEdit {
    /// An edit to a line, which changes nothing until you set a field.
    pub fn new(line: u32) -> Self {
        Self {
            line,
            text: None,
            delivery_style: None,
            voice_instructions: None,
        }
    }

    /// New wording.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// For example `angry` or `whisper`.
    pub fn delivery_style(mut self, style: impl Into<String>) -> Self {
        self.delivery_style = Some(Some(style.into()));
        self
    }

    /// Removes the line's delivery style.
    pub fn clear_delivery_style(mut self) -> Self {
        self.delivery_style = Some(None);
        self
    }

    /// How to say it, in plain words.
    pub fn voice_instructions(mut self, instructions: impl Into<String>) -> Self {
        self.voice_instructions = Some(Some(instructions.into()));
        self
    }

    /// Removes the line's voice instructions.
    pub fn clear_voice_instructions(mut self) -> Self {
        self.voice_instructions = Some(None);
        self
    }
}

/// The answer to editing lines.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct LinesEdited {
    /// The dub.
    pub dub_id: String,
    /// The lines being voiced again.
    pub lines: Vec<u32>,
    /// `revoicing`.
    pub status: String,
}

// ------------------------------------------------------ languages and usage

/// The languages a dub can be made in.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Languages {
    /// Codes such as `fr`.
    pub target_languages: Vec<String>,
}

/// Whether a key makes real or test dubs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KeyMode {
    /// A live key.
    Live,
    /// A test key: free, watermarked, limited to a preview.
    Test,
    /// A mode this version of the crate doesn't know.
    #[serde(other)]
    Unknown,
}

/// What the key can do now.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct Usage {
    /// The account's plan.
    #[serde(default)]
    pub plan: Option<String>,
    /// Live or test.
    pub key_mode: KeyMode,
    /// Minutes left on the plan.
    #[serde(default)]
    pub minutes_remaining: Option<f64>,
    /// Whether the plan has no minute limit.
    pub unlimited: bool,
    /// How many seconds a test dub covers.
    pub test_preview_seconds: u32,
}

/// How many dubs there are in each state.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct DubCounts {
    /// Finished.
    pub completed: u32,
    /// Still running.
    pub in_progress: u32,
    /// Failed or cancelled. Their minutes were returned.
    pub failed: u32,
}

/// Minutes and dubs, used for the total and for each key and language.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct UsageCounts {
    /// Minutes used by completed dubs. Test dubs are free.
    pub minutes: f64,
    /// Minutes reserved by dubs that haven't finished.
    pub minutes_in_progress: f64,
    /// How many dubs.
    pub dubs: DubCounts,
}

/// One API key's usage.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct KeyUsage {
    /// The key's id.
    #[serde(default)]
    pub key_id: Option<String>,
    /// Live or test.
    pub mode: KeyMode,
    /// Its usage.
    #[serde(flatten)]
    pub usage: UsageCounts,
}

/// One target language's usage.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct LanguageUsage {
    /// The language.
    #[serde(default)]
    pub target_language: Option<String>,
    /// Its usage.
    #[serde(flatten)]
    pub usage: UsageCounts,
}

/// One month of the account's API usage. A dub counts in the month it started (UTC).
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct UsageReport {
    /// `YYYY-MM`.
    pub month: String,
    /// The first moment of the month.
    pub period_start: String,
    /// The first moment after it.
    pub period_end: String,
    /// The month's totals.
    #[serde(flatten)]
    pub totals: UsageCounts,
    /// Per key.
    pub keys: Vec<KeyUsage>,
    /// Per target language.
    pub languages: Vec<LanguageUsage>,
}

// ------------------------------------------------------------- webhooks

/// A place Reedapt sends events to.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WebhookEndpoint {
    /// The endpoint's id.
    pub id: String,
    /// Where events are sent.
    pub url: String,
    /// Your note.
    #[serde(default)]
    pub description: Option<String>,
    /// The event types it receives.
    pub events: Vec<String>,
    /// When it was added.
    pub created_at: String,
    /// Only on the endpoint returned by `create`. Keep it to check signatures.
    #[serde(default)]
    pub secret: Option<String>,
}

/// Your webhook endpoints.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WebhookEndpointList {
    /// The endpoints.
    pub data: Vec<WebhookEndpoint>,
}

/// Parameters for [`WebhookEndpoints::create`](crate::resources::WebhookEndpoints::create).
#[derive(Debug, Clone, Serialize)]
#[non_exhaustive]
pub struct CreateWebhookEndpointParams {
    /// Where to send events.
    pub url: String,
    /// A note, up to 255 characters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl CreateWebhookEndpointParams {
    /// An endpoint at `url`.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            description: None,
        }
    }

    /// A note, up to 255 characters.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// One attempt, or series of attempts, to deliver an event.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WebhookDelivery {
    /// The delivery's id.
    pub id: String,
    /// The event's id. A resend keeps it.
    pub event_id: String,
    /// For example `dub.completed`.
    pub event_type: String,
    /// `pending`, `delivered` or `failed`; the API may add more.
    pub status: String,
    /// How many attempts have been made.
    pub attempts: u32,
    /// What your server last answered.
    #[serde(default)]
    pub last_status_code: Option<u16>,
    /// What went wrong last.
    #[serde(default)]
    pub last_error: Option<String>,
    /// When the next attempt is due.
    #[serde(default)]
    pub next_attempt_at: Option<String>,
    /// When the delivery was made.
    pub created_at: String,
    /// When it succeeded.
    #[serde(default)]
    pub delivered_at: Option<String>,
}

/// Recent deliveries to an endpoint.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct WebhookDeliveryList {
    /// The deliveries.
    pub data: Vec<WebhookDelivery>,
}

/// A batch of dubs started together.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct BatchSummary {
    /// The batch's id.
    pub batch_id: String,
    /// Where it is.
    pub status: String,
    /// How many videos it holds.
    pub video_count: u32,
    /// How many dubs it holds.
    pub jobs_total: u32,
    /// How many are done.
    pub jobs_done: u32,
    /// How many failed.
    pub jobs_failed: u32,
    /// The dubs.
    pub dubs: Vec<Dub>,
}
