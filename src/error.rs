//! Everything this crate can fail with.

use std::time::Duration;

use serde::Deserialize;

/// A field the API found a problem with, listed in [`Error::Api::fields`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct FieldProblem {
    /// The name of the field.
    pub field: String,
    /// What is wrong with it.
    pub message: String,
}

/// The `error` object in an API error response.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct ErrorBody {
    pub code: Option<String>,
    pub message: Option<String>,
    pub request_id: Option<String>,
    pub fields: Option<Vec<FieldProblem>>,
}

/// The result of every call in this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// What went wrong with a call.
///
/// Branch on [`Error::Api::code`] (a stable string such as `NOT_FOUND`), never on the
/// message. New variants may be added, so match with a wildcard arm.
///
/// ```
/// # fn handle(err: reedapt::Error) {
/// match &err {
///     reedapt::Error::RateLimited { code, retry_after, .. } => {
///         eprintln!("slow down ({code}), retry after {retry_after:?}");
///     }
///     reedapt::Error::Api { status: 404, .. } => eprintln!("no such object"),
///     other => eprintln!("{other}"),
/// }
/// # }
/// ```
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The API answered with an error other than a rate limit.
    #[error("{code}: {message}")]
    Api {
        /// The HTTP status: 401 for a bad key, 404 for a missing object, 409 for a
        /// conflict, 400/411/413/422 for a request that can't be done as asked, 5xx for
        /// a problem on Reedapt's side.
        status: u16,
        /// A stable code such as `NOT_FOUND`. `HTTP_<status>` when the body had none.
        code: String,
        /// A plain-English explanation.
        message: String,
        /// Quote this to support.
        request_id: Option<String>,
        /// The fields the API found invalid, if it said which.
        fields: Vec<FieldProblem>,
    },

    /// 429: too many requests, or a limit such as dubs running at once.
    #[error("{code}: {message}")]
    RateLimited {
        /// A stable code such as `RATE_LIMITED` or `TOO_MANY_DUBS_RUNNING`.
        code: String,
        /// A plain-English explanation.
        message: String,
        /// Quote this to support.
        request_id: Option<String>,
        /// How long the API asked you to wait, from the `Retry-After` header.
        retry_after: Option<Duration>,
    },

    /// The request never got an answer: the network failed or it timed out.
    #[error("request failed: {0}")]
    Transport(#[source] reqwest::Error),

    /// The API answered, but the answer wasn't what this crate expected.
    #[error("could not read the API's answer: {0}")]
    Decode(#[source] serde_json::Error),

    /// The client was set up wrongly, for example with no API key.
    #[error("{0}")]
    Config(String),

    /// A file could not be read, or an upload part could not be sent to storage.
    #[error("upload failed: {0}")]
    Upload(String),

    /// A `wait_until_…` helper ran out of time, or the thing it waited on failed.
    #[error("{0}")]
    Wait(String),

    /// A local file could not be read.
    #[error("could not read the file: {0}")]
    Io(#[from] std::io::Error),

    /// A webhook's signature didn't check out.
    #[error(transparent)]
    Webhook(#[from] crate::webhook::WebhookError),
}

impl Error {
    /// The HTTP status, when the API answered.
    pub fn status(&self) -> Option<u16> {
        match self {
            Error::Api { status, .. } => Some(*status),
            Error::RateLimited { .. } => Some(429),
            _ => None,
        }
    }

    /// The API's error code, when it answered.
    pub fn code(&self) -> Option<&str> {
        match self {
            Error::Api { code, .. } | Error::RateLimited { code, .. } => Some(code),
            _ => None,
        }
    }

    /// The request id to quote to support, when the API answered.
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Error::Api { request_id, .. } | Error::RateLimited { request_id, .. } => request_id.as_deref(),
            _ => None,
        }
    }

    /// True for a 401: the key is missing, wrong or revoked.
    pub fn is_authentication(&self) -> bool {
        self.status() == Some(401)
    }

    /// True for a 404: nothing with that id belongs to the account.
    pub fn is_not_found(&self) -> bool {
        self.status() == Some(404)
    }

    /// True for a 409: the request conflicts with the object's state.
    pub fn is_conflict(&self) -> bool {
        self.status() == Some(409)
    }

    /// True when the request timed out.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Error::Transport(e) if e.is_timeout())
    }

    pub(crate) fn from_response(
        status: u16,
        body: Option<ErrorBody>,
        request_id_header: Option<String>,
        retry_after: Option<Duration>,
    ) -> Error {
        let body = body.unwrap_or_default();
        let code = body.code.unwrap_or_else(|| format!("HTTP_{status}"));
        let message = body
            .message
            .unwrap_or_else(|| format!("The API answered {status}."));
        let request_id = body.request_id.or(request_id_header);
        if status == 429 {
            Error::RateLimited {
                code,
                message,
                request_id,
                retry_after,
            }
        } else {
            Error::Api {
                status,
                code,
                message,
                request_id,
                fields: body.fields.unwrap_or_default(),
            }
        }
    }
}
