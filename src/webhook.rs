//! Checking that a webhook delivery really came from Reedapt.
//!
//! Every delivery carries a `Reedapt-Signature` header such as
//! `t=1700000000,v1=<hex>`. The signature is an HMAC-SHA256, keyed with the endpoint's
//! `whsec_` secret, of `"<t>.<raw body>"`. Check it against the body exactly as it
//! arrived: re-encoded JSON won't match.
//!
//! ```
//! use hmac::{Hmac, Mac};
//! use sha2::Sha256;
//!
//! let secret = "whsec_test";
//! let body = r#"{"id":"evt_1","type":"webhook.test","created_at":"2026-11-22T00:00:00Z","data":{}}"#;
//!
//! // What Reedapt sends (shown here so the example runs):
//! let t = std::time::SystemTime::now()
//!     .duration_since(std::time::UNIX_EPOCH)
//!     .unwrap()
//!     .as_secs();
//! let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
//! mac.update(format!("{t}.{body}").as_bytes());
//! let sig: String = mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
//! let header = format!("t={t},v1={sig}");
//!
//! // What your handler does:
//! let event = reedapt::webhook::construct_event(body, &header, secret)?;
//! assert_eq!(event.id, "evt_1");
//! # Ok::<(), reedapt::Error>(())
//! ```

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::error::{Error, Result};
use crate::types::{BatchSummary, Dub, Quality};

/// How old a signature may be by default: five minutes.
pub const DEFAULT_TOLERANCE: Duration = Duration::from_secs(300);

/// Why a delivery was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WebhookError {
    /// There was no `Reedapt-Signature` header.
    #[error("The Reedapt-Signature header is missing.")]
    MissingHeader,
    /// The header isn't `t=<seconds>,v1=<hex>`.
    #[error("The Reedapt-Signature header is malformed.")]
    MalformedHeader,
    /// The timestamp is further from now than the tolerance.
    #[error("The signature is too old. Check your server's clock.")]
    TooOld,
    /// No signature matched.
    #[error("The signature doesn't match. Use the endpoint's secret and the raw body, not re-encoded JSON.")]
    Mismatch,
}

/// Checks a delivery's `Reedapt-Signature` header against the raw request body, exactly
/// as it arrived. Accepts signatures up to five minutes old.
///
/// The comparison takes the same time whether or not the signature is close.
pub fn verify(payload: impl AsRef<[u8]>, header: &str, secret: &str) -> Result<(), WebhookError> {
    verify_with_tolerance(payload, header, secret, DEFAULT_TOLERANCE)
}

/// [`verify`] with your own limit on how old a signature may be.
pub fn verify_with_tolerance(
    payload: impl AsRef<[u8]>,
    header: &str,
    secret: &str,
    tolerance: Duration,
) -> Result<(), WebhookError> {
    if header.trim().is_empty() {
        return Err(WebhookError::MissingHeader);
    }
    let mut timestamp: Option<&str> = None;
    let mut signatures: Vec<&str> = Vec::new();
    for item in header.split(',') {
        let Some((name, value)) = item.split_once('=') else {
            continue;
        };
        match name.trim() {
            "t" => timestamp = Some(value.trim()),
            "v1" => signatures.push(value.trim()),
            _ => {}
        }
    }
    let timestamp = timestamp
        .filter(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
        .ok_or(WebhookError::MalformedHeader)?;
    if signatures.is_empty() {
        return Err(WebhookError::MalformedHeader);
    }
    let signed_at: f64 = timestamp.parse().map_err(|_| WebhookError::MalformedHeader)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    if (now - signed_at).abs() > tolerance.as_secs_f64() {
        return Err(WebhookError::TooOld);
    }

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(payload.as_ref());

    // `verify_slice` compares in constant time. Check every candidate so the time doesn't
    // depend on which one matched.
    let mut matched = false;
    for given in signatures {
        if let Some(bytes) = decode_hex(given) {
            matched |= mac.clone().verify_slice(&bytes).is_ok();
        }
    }
    if matched {
        Ok(())
    } else {
        Err(WebhookError::Mismatch)
    }
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// An event Reedapt sent.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Event {
    /// The event's id. An event can arrive more than once; skip ids you've handled.
    pub id: String,
    /// When it happened (RFC 3339).
    pub created_at: String,
    /// What happened.
    pub data: EventData,
}

/// What an [`Event`] is about. New event types may be added; handle the ones you know
/// and ignore the rest.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum EventData {
    /// `dub.completed`
    DubCompleted {
        /// The finished dub.
        dub: Box<Dub>,
    },
    /// `dub.failed`
    DubFailed {
        /// The dub that failed.
        dub: Box<Dub>,
    },
    /// `dub.checked`: a few minutes after completion, on plans where Reedapt checks dubs.
    DubChecked {
        /// The dub.
        dub: Box<Dub>,
        /// The lines Reedapt fixed and the ones left to review.
        quality: Box<Quality>,
    },
    /// `batch.completed`
    BatchCompleted {
        /// The batch.
        batch: Box<BatchSummary>,
    },
    /// `webhook.test`: sent by [`WebhookEndpoints::send_test`](crate::resources::WebhookEndpoints::send_test).
    WebhookTest,
    /// A type this version of the crate doesn't know.
    Unknown {
        /// For example `dub.something_new`.
        event_type: String,
        /// Its payload, as sent.
        data: serde_json::Value,
    },
}

impl Event {
    /// The event's type as the API names it, such as `dub.completed`.
    pub fn event_type(&self) -> &str {
        match &self.data {
            EventData::DubCompleted { .. } => "dub.completed",
            EventData::DubFailed { .. } => "dub.failed",
            EventData::DubChecked { .. } => "dub.checked",
            EventData::BatchCompleted { .. } => "batch.completed",
            EventData::WebhookTest => "webhook.test",
            EventData::Unknown { event_type, .. } => event_type,
        }
    }
}

#[derive(Deserialize)]
struct Raw {
    id: String,
    #[serde(rename = "type")]
    event_type: String,
    created_at: String,
    #[serde(default)]
    data: serde_json::Value,
}

#[derive(Deserialize)]
struct DubData {
    dub: Dub,
}

#[derive(Deserialize)]
struct CheckedData {
    dub: Dub,
    quality: Quality,
}

#[derive(Deserialize)]
struct BatchData {
    batch: BatchSummary,
}

/// Verifies a delivery and returns its parsed event. Accepts signatures up to five
/// minutes old.
pub fn construct_event(payload: impl AsRef<[u8]>, header: &str, secret: &str) -> Result<Event> {
    construct_event_with_tolerance(payload, header, secret, DEFAULT_TOLERANCE)
}

/// [`construct_event`] with your own limit on how old a signature may be.
pub fn construct_event_with_tolerance(
    payload: impl AsRef<[u8]>,
    header: &str,
    secret: &str,
    tolerance: Duration,
) -> Result<Event> {
    verify_with_tolerance(payload.as_ref(), header, secret, tolerance)?;
    parse_event(payload.as_ref())
}

/// Parses an event without checking its signature. Only for payloads you trust, such as
/// stored events that were verified when they arrived.
pub fn parse_event(payload: &[u8]) -> Result<Event> {
    let raw: Raw = serde_json::from_slice(payload).map_err(Error::Decode)?;
    let data = match raw.event_type.as_str() {
        "dub.completed" => {
            from_data::<DubData>(raw.data).map(|d| EventData::DubCompleted { dub: Box::new(d.dub) })
        }
        "dub.failed" => from_data::<DubData>(raw.data).map(|d| EventData::DubFailed { dub: Box::new(d.dub) }),
        "dub.checked" => from_data::<CheckedData>(raw.data).map(|d| EventData::DubChecked {
            dub: Box::new(d.dub),
            quality: Box::new(d.quality),
        }),
        "batch.completed" => from_data::<BatchData>(raw.data).map(|d| EventData::BatchCompleted {
            batch: Box::new(d.batch),
        }),
        "webhook.test" => Ok(EventData::WebhookTest),
        _ => Ok(EventData::Unknown {
            event_type: raw.event_type,
            data: raw.data,
        }),
    }?;
    Ok(Event {
        id: raw.id,
        created_at: raw.created_at,
        data,
    })
}

fn from_data<T: serde::de::DeserializeOwned>(data: serde_json::Value) -> Result<T> {
    serde_json::from_value(data).map_err(Error::Decode)
}
