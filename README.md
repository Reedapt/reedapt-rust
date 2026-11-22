# Reedapt Rust library

The official Rust library for the [Reedapt API](https://docs.reedapt.com). Upload video or audio, dub it into other languages in each speaker's own voice, and get the files back.

## Install

```bash
cargo add reedapt
```

Rust 1.83 or later. It is async, on [tokio](https://tokio.rs), and speaks HTTPS with rustls, so it needs no system OpenSSL. For programs without async, enable the `blocking` feature:

```toml
reedapt = { version = "0.2", features = ["blocking"] }
```

## Quickstart

Make a key under **Developers** in [Reedapt](https://dub.reedapt.com/developers) and keep it in `REEDAPT_API_KEY`. Test keys are free and dub the first 60 seconds with a watermark.

```rust
use reedapt::types::{DubParams, DubStatus};

#[tokio::main]
async fn main() -> reedapt::Result<()> {
    let client = reedapt::Client::from_env()?; // reads REEDAPT_API_KEY

    let created = client
        .dub_file("interview.mp4", DubParams::new("en", ["fr", "es"]))
        .await?;

    for started in created.dubs {
        let dub = client.dubs().wait_until_done(&started.id, Default::default()).await?;
        if dub.status == DubStatus::Completed {
            let files = client.dubs().files(&dub.id).await?;
            println!("{:?}: {}", dub.target_language, files.files[0].url);
        } else {
            println!("{:?} failed: {:?}", dub.target_language, dub.error);
        }
    }
    Ok(())
}
```

`dub_file` takes a path. `dub_url` takes a public `https://` link that Reedapt downloads itself. In production, use [webhooks](#webhooks) instead of `wait_until_done`.

## Uploads

`uploads().upload_file` sends the file straight to storage in parts, four at a time, signs more links as it needs them, and retries parts that fail. Only the parts being sent are held in memory. It also takes bytes and any `AsyncRead`:

```rust
use reedapt::resources::UploadOptions;
use reedapt::types::CreateDubParams;

let upload = client
    .uploads()
    .upload_file_with(
        "film.mov",
        UploadOptions::new()
            .concurrency(4)
            .on_progress(|p| println!("{}%", 100 * p.uploaded_bytes / p.total_bytes)),
    )
    .await?;

// An AsyncRead needs its exact size, because the API divides the file into parts up front.
let upload = client.uploads().upload_reader(reader, "film.mov", size, UploadOptions::new()).await?;

let created = client
    .dubs()
    .create(
        CreateDubParams::new(upload.id, "en", ["yo"])
            .metadata("order_id", "1042")
            .instructions("Keep the speaker's name, Adaeze, as it is. Warm, informal tone."), // paid plans
    )
    .await?;
```

For a file that's already online:

```rust
use reedapt::resources::FromUrlOptions;

let upload = client.uploads().from_url("https://example.com/media/talk.mp4", FromUrlOptions::new()).await?;
```

## Dubs

```rust
client.dubs().retrieve("job_3f9a2c7e1b4d").await?;
client.dubs().list(ListDubsParams::new().limit(20)).await?;
client.dubs().files("job_3f9a2c7e1b4d").await?;      // links last an hour
client.dubs().cancel("job_3f9a2c7e1b4d").await?;     // minutes are returned
client.dubs().transcript("job_3f9a2c7e1b4d").await?;  // every line, with timing, speaker and translation
client.dubs().subtitles("job_3f9a2c7e1b4d", SubtitlesParams::new().format(SubtitleFormat::Vtt)).await?;
client.dubs().quality("job_3f9a2c7e1b4d").await?;     // Reedapt's check: lines fixed and left to review

client.languages().list().await?;                     // target_languages
client.usage().retrieve().await?;                     // plan, key mode and minutes left
client.usage().report(Some("2026-10")).await?;        // minutes and dubs per key and per language
```

`list_all` is a `futures::Stream` that fetches pages as you read it:

```rust
use futures_util::TryStreamExt;

let mut dubs = std::pin::pin!(client.dubs().list_all(None));
while let Some(dub) = dubs.try_next().await? {
    println!("{} {:?}", dub.id, dub.status);
}
```

## Ask Reedapt for changes

Early access: this is released to some paid accounts first, and other accounts get a `403` with the code `NOT_AVAILABLE`. Describe a change in plain words and Reedapt either answers, proposes a change, or makes it. With `Autonomy::Ask` (the default) nothing changes until you say so:

```rust
use reedapt::types::{CreateRunParams, RunChoice, RunStatus};

let mut run = client
    .dubs()
    .requests()
    .create("job_3f9a2c7e1b4d", CreateRunParams::new("Is line 3 natural? If not, make it sound calmer."))
    .await?;
println!("{}", run.reply);

if run.status == RunStatus::NeedsInput {
    if let Some(p) = &run.proposal {
        println!("{} ({} lines)", p.summary, p.lines);
    }
    run = client.runs().answer(&run.id, RunChoice::Apply).await?; // or RunChoice::Decline
}
```

You can also read and edit lines yourself. Edited lines are voiced again in the background:

```rust
use reedapt::types::LineEdit;

let lines = client.dubs().lines().list("job_3f9a2c7e1b4d").await?;
client
    .dubs()
    .lines()
    .edit("job_3f9a2c7e1b4d", vec![LineEdit::new(3).text("Bienvenue chez nous.").delivery_style("warm")])
    .await?; // status "revoicing"
```

A request counts against your plan's allowance for these requests; when it's used up you get `AGENT_ALLOWANCE_USED` (429) until it renews.

## Webhooks

Add an endpoint under **Developers**, or with `client.webhook_endpoints().create(...)`, and keep its `whsec_` secret. `resend_delivery(endpoint_id, delivery_id)` sends a past delivery again. Check every delivery against the raw body, exactly as it arrived:

```rust
use reedapt::webhook::{self, EventData};

// In your handler: `body` is the raw request body, `signature` the Reedapt-Signature header.
let event = match webhook::construct_event(body, signature, &secret) {
    Ok(event) => event,
    Err(_) => return StatusCode::BAD_REQUEST,
};
match event.data {
    EventData::DubCompleted { dub } => { /* queue the work, then answer quickly */ }
    EventData::DubChecked { quality, .. } => { /* quality.to_review lines are left to review */ }
    _ => {} // Ignore types you don't know; every endpoint gets every event.
}
```

`webhook::verify(payload, header, secret)` checks the signature alone; `verify_with_tolerance` and `construct_event_with_tolerance` change how old a signature may be (default 5 minutes). The check is an HMAC-SHA256 compared in constant time, the same algorithm as the Node.js and Python libraries.

An event can arrive more than once, so skip any `event.id` you've handled.

## Errors

Every call returns `reedapt::Result<T>`. API errors carry the HTTP `status`, a stable `code`, a plain-English `message` and the `request_id` to quote to support:

```rust
use reedapt::Error;

match client.dubs().create(params).await {
    Ok(created) => println!("{} dubs started", created.dubs.len()),
    Err(Error::RateLimited { code, .. }) if code == "TOO_MANY_DUBS_RUNNING" => {
        // Start more once some finish.
    }
    Err(e) => eprintln!("{e} (status {:?}, request {:?})", e.status(), e.request_id()),
}
```

| Variant | When |
| --- | --- |
| `Error::Api { status, code, message, request_id, fields }` | The API refused: 401 bad key, 404, 409 conflict, 400/411/413/422 invalid request (`fields` lists invalid fields), 5xx. |
| `Error::RateLimited { code, message, request_id, retry_after }` | 429. |
| `Error::Transport` | No answer arrived: the network failed or the request timed out (`is_timeout()`). |
| `Error::Decode` | The answer wasn't the JSON this library expects. |
| `Error::Upload` | A file couldn't be read, or a part couldn't be sent to storage. |
| `Error::Wait` | A `wait_until_…` helper ran out of time, or the upload failed. |
| `Error::Webhook` | A delivery's signature didn't check out. |
| `Error::Config` | The client was set up wrongly, such as no API key. |

The enum is `#[non_exhaustive]`, so keep a wildcard arm. The [error codes](https://docs.reedapt.com/errors) are listed in the docs.

Response structs and enums are `#[non_exhaustive]` too, and enums have an `Unknown` variant, so a newer API doesn't break an older build.

## Retries and repeated requests

Network failures, `500`, `502`, `503`, `504` and per-minute rate limits are retried twice, with exponential backoff, honouring `Retry-After`. Creating uploads and dubs always sends an `Idempotency-Key`, so a retry never makes a second dub. Pass your own to make a repeat safe across restarts:

```rust
use reedapt::RequestOptions;

client
    .with_options(RequestOptions::new().idempotency_key(format!("order-{}", order.id)))
    .dubs()
    .create(params)
    .await?;
```

## Configuration

```rust
use std::time::Duration;

let client = reedapt::Client::builder()
    .api_key(std::env::var("REEDAPT_API_KEY").unwrap())
    .timeout(Duration::from_secs(60)) // per attempt
    .max_retries(2)
    // .base_url("https://api.reedapt.com/v1")
    // .http_client(my_reqwest_client) // proxies, extra root certificates, a shared pool
    .build()?;

// Per call:
client
    .with_options(RequestOptions::new().timeout(Duration::from_secs(10)).max_retries(0))
    .dubs()
    .retrieve(id)
    .await?;
```

The client is cheap to clone and safe to share between tasks. Cancel a call by dropping its future, for example with `tokio::time::timeout` or `tokio::select!`.

A custom `reqwest::Client` must come from reqwest 0.12, the version this crate uses.

Keep keys on your server. Never put one in a web page or an app people can download.

## Blocking

```rust
let client = reedapt::blocking::Client::from_env()?;
let created = client.dub_file("interview.mp4", DubParams::new("en", ["fr"]))?;
for dub in client.dubs().list_all(None) {
    println!("{}", dub?.id);
}
```

It has the same resources and methods without `.await`, and `list_all` is an `Iterator`. Don't call it from inside an async runtime.

## Links

- [Documentation](https://docs.reedapt.com)
- [API reference](https://docs.reedapt.com/api-reference/introduction)
- [Status](https://status.reedapt.com)
- Support: support@reedapt.com
