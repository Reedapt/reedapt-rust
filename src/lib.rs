//! The official Rust library for the [Reedapt API](https://docs.reedapt.com). Upload video
//! or audio, dub it into other languages in each speaker's own voice, and get the files
//! back.
//!
//! It is async (on [tokio](https://tokio.rs)) and talks HTTPS with rustls, so it needs no
//! system OpenSSL. Enable the `blocking` feature for [`blocking::Client`], which works
//! without an async runtime of your own.
//!
//! # Quickstart
//!
//! Make a key under **Developers** in [Reedapt](https://dub.reedapt.com/developers) and
//! keep it in `REEDAPT_API_KEY`. Test keys are free and dub the first 60 seconds with a
//! watermark.
//!
//! ```no_run
//! use reedapt::types::{DubParams, DubStatus};
//!
//! #[tokio::main]
//! async fn main() -> reedapt::Result<()> {
//!     let client = reedapt::Client::from_env()?;
//!
//!     // Sends the file in parts, then starts one dub per language.
//!     let created = client
//!         .dub_file("interview.mp4", DubParams::new("en", ["fr", "es"]))
//!         .await?;
//!
//!     for started in created.dubs {
//!         let dub = client
//!             .dubs()
//!             .wait_until_done(&started.id, Default::default())
//!             .await?;
//!         if dub.status == DubStatus::Completed {
//!             let files = client.dubs().files(&dub.id).await?;
//!             println!("{:?}: {}", dub.target_language, files.files[0].url);
//!         }
//!     }
//!     Ok(())
//! }
//! ```
//!
//! In production, use [webhooks](webhook) instead of waiting.
//!
//! # What's here
//!
//! | | |
//! | --- | --- |
//! | [`Client`] | Build one with [`Client::new`] or [`Client::builder`]. Cheap to clone. |
//! | [`Client::uploads`] | [`upload_file`](resources::Uploads::upload_file), [`upload_reader`](resources::Uploads::upload_reader), [`from_url`](resources::Uploads::from_url) |
//! | [`Client::dubs`] | create, retrieve, list, [`list_all`](resources::Dubs::list_all) (a `Stream`), cancel, files, transcript, subtitles, quality |
//! | [`Client::runs`], `dubs().requests()`, `dubs().lines()` | Asking for changes to a dub. Early access. |
//! | [`Client::webhook_endpoints`] | create, list, delete, send_test, deliveries, resend_delivery |
//! | [`Client::languages`], [`Client::usage`] | languages, usage and a monthly usage report |
//! | [`webhook`] | Signature checks for deliveries |
//! | [`types`] | Every request and response type |
//!
//! # Errors
//!
//! Every call returns [`Result<T>`](Result) with an [`Error`]. API errors carry the HTTP
//! status, a stable `code` and the `request_id` to quote to support:
//!
//! ```no_run
//! # async fn run(client: reedapt::Client, params: reedapt::types::CreateDubParams) {
//! match client.dubs().create(params).await {
//!     Ok(created) => println!("{} dubs started", created.dubs.len()),
//!     Err(reedapt::Error::RateLimited { code, .. }) if code == "TOO_MANY_DUBS_RUNNING" => {
//!         // Start more once some finish.
//!     }
//!     Err(e) => eprintln!("{e} (request {:?})", e.request_id()),
//! }
//! # }
//! ```
//!
//! # Retries and repeated requests
//!
//! Network failures, `500`, `502`, `503`, `504` and per-minute rate limits are retried
//! twice with exponential backoff, honouring `Retry-After`. Creating uploads and dubs always
//! sends an `Idempotency-Key`, so a retry never makes a second dub. Pass your own to make a
//! repeat safe across restarts:
//!
//! ```no_run
//! # async fn run(client: reedapt::Client, params: reedapt::types::CreateDubParams) -> reedapt::Result<()> {
//! use reedapt::RequestOptions;
//!
//! client
//!     .with_options(RequestOptions::new().idempotency_key("order-1042"))
//!     .dubs()
//!     .create(params)
//!     .await?;
//! # Ok(()) }
//! ```
//!
//! Keep keys on your server. Never put one in a web page or an app people can download.

#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs)]

#[cfg(feature = "blocking")]
#[cfg_attr(docsrs, doc(cfg(feature = "blocking")))]
pub mod blocking;
mod client;
mod error;
pub mod resources;
pub mod types;
pub mod webhook;

pub use client::{Client, ClientBuilder, RequestOptions, DEFAULT_BASE_URL, VERSION};
pub use error::{Error, FieldProblem, Result};
