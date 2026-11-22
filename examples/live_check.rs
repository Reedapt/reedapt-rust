//! Read-only calls against the API with the key in `REEDAPT_API_KEY`. Starts no dubs.
//!
//! ```text
//! REEDAPT_API_KEY=rk_test_... cargo run --example live_check
//! ```

use futures_util::{StreamExt, TryStreamExt};
use reedapt::types::ListDubsParams;

#[tokio::main]
async fn main() -> reedapt::Result<()> {
    let client = reedapt::Client::from_env()?;

    let languages = client.languages().list().await?;
    println!(
        "languages: {} targets, first {:?}",
        languages.target_languages.len(),
        languages.target_languages.first()
    );

    let usage = client.usage().retrieve().await?;
    println!(
        "usage: mode {:?}, plan {:?}, unlimited {}",
        usage.key_mode, usage.plan, usage.unlimited
    );

    let report = client.usage().report(None).await?;
    println!(
        "report {}: {} minutes, {} dubs completed",
        report.month, report.totals.minutes, report.totals.dubs.completed
    );

    let page = client.dubs().list(ListDubsParams::new().limit(5)).await?;
    println!(
        "dubs: {} on the first page, more: {}",
        page.data.len(),
        page.has_more
    );
    if let Some(first) = page.data.first() {
        println!(
            "  newest: status {:?}, target {:?}",
            first.status, first.target_language
        );
    }

    // Pages of 3, so seven dubs cross three requests.
    let ids: Vec<String> = client
        .dubs()
        .list_all(Some(3))
        .take(7)
        .map_ok(|d| d.id)
        .try_collect()
        .await?;
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    println!(
        "list_all: {} dubs across pages, {} unique",
        ids.len(),
        unique.len()
    );

    let endpoints = client.webhook_endpoints().list().await?;
    println!("webhook endpoints: {}", endpoints.data.len());

    // A missing object comes back as a typed error with a code and request id.
    match client.dubs().retrieve("job_does_not_exist").await {
        Err(e) if e.is_not_found() => println!(
            "missing dub: {} (request id present: {})",
            e.code().unwrap_or("?"),
            e.request_id().is_some()
        ),
        other => println!("unexpected: {other:?}"),
    }
    Ok(())
}
