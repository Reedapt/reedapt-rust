//! One type per part of the API, reached from a [`Client`](crate::Client).

mod dubs;
mod misc;
mod uploads;
mod webhook_endpoints;

pub use dubs::{DubLinesApi, DubRequests, Dubs, Runs, WaitForDubOptions};
pub use misc::{LanguagesApi, UsageApi};
pub use uploads::{FromUrlOptions, UploadOptions, UploadProgress, Uploads, WaitOptions};
pub use webhook_endpoints::WebhookEndpoints;

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};

const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// Makes an id safe to place in a URL path.
pub(crate) fn seg(id: &str) -> String {
    utf8_percent_encode(id, SEGMENT).to_string()
}
