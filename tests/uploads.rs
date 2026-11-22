mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use common::{client, dub, error, upload};
use reedapt::resources::{FromUrlOptions, UploadOptions, WaitOptions};
use reedapt::types::*;
use reedapt::{Error, RequestOptions};
use serde_json::json;
use std::time::Duration;
use wiremock::matchers::{body_bytes, body_json, header, header_exists, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

const DATA: &[u8] = b"0123456789"; // three parts of 4, 4 and 2 bytes

async fn mount_part(server: &MockServer, id: &str, n: u32, bytes: &'static [u8]) {
    Mock::given(method("PUT"))
        .and(path(format!("/storage/{id}/{n}")))
        .and(body_bytes(bytes))
        .respond_with(ResponseTemplate::new(200).insert_header("etag", format!("\"etag-{n}\"").as_str()))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_create(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .and(header_exists("idempotency-key"))
        .and(body_json(json!({"filename": "talk.mp4", "size": 10})))
        .respond_with(ResponseTemplate::new(200).set_body_json(upload("up_1", 10, 4, 3, "uploading", server)))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_sign_third(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/uploads/up_1/parts"))
        .and(body_json(json!({"part_numbers": [3]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"parts": [{"part_number": 3, "url": format!("{}/storage/up_1/3", server.uri())}]}),
        ))
        .expect(1)
        .mount(server)
        .await;
}

async fn mount_complete(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/v1/uploads/up_1/complete"))
        .and(body_json(json!({"parts": [
            {"part_number": 1, "etag": "\"etag-1\""},
            {"part_number": 2, "etag": "\"etag-2\""},
            {"part_number": 3, "etag": "\"etag-3\""}
        ]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(upload("up_1", 10, 4, 3, "uploaded", server)))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn bytes_are_sent_in_parts_signed_as_needed_and_completed() {
    let server = MockServer::start().await;
    mount_create(&server).await;
    mount_sign_third(&server).await;
    mount_part(&server, "up_1", 1, b"0123").await;
    mount_part(&server, "up_1", 2, b"4567").await;
    mount_part(&server, "up_1", 3, b"89").await;
    mount_complete(&server).await;

    let progress = Arc::new(AtomicU64::new(0));
    let seen = progress.clone();
    let up = client(&server)
        .uploads()
        .upload_bytes(
            DATA,
            "talk.mp4",
            UploadOptions::new().concurrency(2).on_progress(move |p| {
                assert_eq!(p.total_bytes, 10);
                seen.store(p.uploaded_bytes, Ordering::SeqCst);
            }),
        )
        .await
        .unwrap();
    assert_eq!(up.status, UploadStatus::Uploaded);
    assert_eq!(progress.load(Ordering::SeqCst), 10);

    // Storage never sees the API key.
    for r in server.received_requests().await.unwrap() {
        if r.url.path().starts_with("/storage") {
            assert!(r.headers.get("authorization").is_none());
        }
    }
}

#[tokio::test]
async fn a_file_on_disk_is_read_in_parts() {
    let dir = std::env::temp_dir().join(format!("reedapt-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("talk.mp4");
    std::fs::write(&file, DATA).unwrap();

    let server = MockServer::start().await;
    mount_create(&server).await;
    mount_sign_third(&server).await;
    mount_part(&server, "up_1", 1, b"0123").await;
    mount_part(&server, "up_1", 2, b"4567").await;
    mount_part(&server, "up_1", 3, b"89").await;
    mount_complete(&server).await;

    let up = client(&server).uploads().upload_file(&file).await.unwrap();
    assert_eq!(up.id, "up_1");
    std::fs::remove_dir_all(dir).unwrap();
}

#[tokio::test]
async fn an_async_reader_works_and_a_short_one_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(upload(
            "up_1",
            10,
            4,
            3,
            "uploading",
            &server,
        )))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads/up_1/parts"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"parts": [{"part_number": 3, "url": format!("{}/storage/up_1/3", server.uri())}]}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(200).insert_header("etag", "\"e\""))
        .mount(&server)
        .await;
    // The reader holds 6 bytes but the caller said 10.
    let err = client(&server)
        .uploads()
        .upload_reader(&b"012345"[..], "talk.mp4", 10, UploadOptions::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Upload(ref m) if m.contains("ended before part 2")),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_empty_file_is_refused_before_any_request() {
    let server = MockServer::start().await;
    let err = client(&server)
        .uploads()
        .upload_bytes(Vec::new(), "a.mp4", UploadOptions::new())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Upload(_)));
    assert!(server.received_requests().await.unwrap().is_empty());
}

struct FailOnce(std::sync::atomic::AtomicBool, u16);

impl Respond for FailOnce {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        if self.0.swap(false, Ordering::SeqCst) {
            ResponseTemplate::new(self.1)
        } else {
            ResponseTemplate::new(200).insert_header("etag", "\"etag-1\"")
        }
    }
}

#[tokio::test]
async fn an_expired_link_is_signed_again() {
    let server = MockServer::start().await;
    let one_part = json!({"id": "up_1", "filename": "a.mp4", "size": 4, "status": "uploading", "part_size": 4, "part_count": 1,
        "parts": [{"part_number": 1, "url": format!("{}/storage/up_1/1", server.uri())}]});
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&one_part))
        .mount(&server)
        .await;
    // The first PUT is refused as expired; the part is signed again and sent to a new link.
    Mock::given(method("PUT"))
        .and(path("/storage/up_1/1"))
        .respond_with(FailOnce(std::sync::atomic::AtomicBool::new(true), 403))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads/up_1/parts"))
        .and(body_json(json!({"part_numbers": [1]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"parts": [{"part_number": 1, "url": format!("{}/storage/up_1/1", server.uri())}]}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads/up_1/complete"))
        .and(body_json(
            json!({"parts": [{"part_number": 1, "etag": "\"etag-1\""}]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(one_part))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .uploads()
        .upload_bytes(&b"0123"[..], "a.mp4", UploadOptions::new())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_failing_part_is_retried_then_reported() {
    let server = MockServer::start().await;
    let one_part = json!({"id": "up_1", "filename": "a.mp4", "size": 4, "status": "uploading", "part_size": 4, "part_count": 1,
        "parts": [{"part_number": 1, "url": format!("{}/storage/up_1/1", server.uri())}]});
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&one_part))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/storage/up_1/1"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await; // not retryable
    let err = client(&server)
        .uploads()
        .upload_bytes(&b"0123"[..], "a.mp4", UploadOptions::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Upload(ref m) if m.contains("Part 1") && m.contains("404")),
        "{err:?}"
    );

    // A 500 from storage is retried (4 attempts in all, with backoff).
    let server = MockServer::start().await;
    let one_part = json!({"id": "up_1", "filename": "a.mp4", "size": 4, "status": "uploading", "part_size": 4, "part_count": 1,
        "parts": [{"part_number": 1, "url": format!("{}/storage/up_1/1", server.uri())}]});
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&one_part))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/storage/up_1/1"))
        .respond_with(FailOnce(std::sync::atomic::AtomicBool::new(true), 500))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads/up_1/complete"))
        .respond_with(ResponseTemplate::new(200).set_body_json(one_part))
        .mount(&server)
        .await;
    client(&server)
        .uploads()
        .upload_bytes(&b"0123"[..], "a.mp4", UploadOptions::new())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_complete_that_already_went_through_is_not_an_error() {
    let server = MockServer::start().await;
    let one_part = json!({"id": "up_1", "filename": "a.mp4", "size": 4, "status": "uploading", "part_size": 4, "part_count": 1,
        "parts": [{"part_number": 1, "url": format!("{}/storage/up_1/1", server.uri())}]});
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&one_part))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .respond_with(ResponseTemplate::new(200).insert_header("etag", "\"e\""))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads/up_1/complete"))
        .respond_with(ResponseTemplate::new(409).set_body_json(error("UPLOAD_COMPLETE", "Already complete.")))
        .expect(1)
        .mount(&server)
        .await;
    let mut done = one_part.clone();
    done["status"] = json!("uploaded");
    Mock::given(method("GET"))
        .and(path("/v1/uploads/up_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(done))
        .expect(1)
        .mount(&server)
        .await;
    let up = client(&server)
        .uploads()
        .upload_bytes(&b"0123"[..], "a.mp4", UploadOptions::new())
        .await
        .unwrap();
    assert_eq!(up.status, UploadStatus::Uploaded);
}

#[tokio::test]
async fn from_url_waits_until_the_download_is_done() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .and(body_json(
            json!({"source_url": "https://example.com/talk.mp4", "filename": "talk.mp4"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(upload("up_2", 0, 4, 0, "fetching", &server)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/uploads/up_2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(upload("up_2", 10, 4, 0, "fetching", &server)))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/uploads/up_2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(upload("up_2", 10, 4, 0, "uploaded", &server)))
        .mount(&server)
        .await;
    let mut options = FromUrlOptions::new().filename("talk.mp4");
    options.wait = WaitOptions::new().poll_interval(Duration::from_millis(10));
    let up = client(&server)
        .uploads()
        .from_url("https://example.com/talk.mp4", options)
        .await
        .unwrap();
    assert_eq!(up.status, UploadStatus::Uploaded);
}

#[tokio::test]
async fn from_url_can_skip_waiting_and_waiting_reports_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .respond_with(ResponseTemplate::new(200).set_body_json(upload("up_2", 0, 4, 0, "fetching", &server)))
        .mount(&server)
        .await;
    let up = client(&server)
        .uploads()
        .from_url("https://example.com/a.mp4", FromUrlOptions::new().no_wait())
        .await
        .unwrap();
    assert_eq!(up.status, UploadStatus::Fetching);

    let mut failed = upload("up_2", 0, 4, 0, "failed", &server);
    failed["error"] = json!("The link needs a sign-in.");
    Mock::given(method("GET"))
        .and(path("/v1/uploads/up_2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(failed))
        .mount(&server)
        .await;
    let err = client(&server)
        .uploads()
        .wait_until_ready("up_2", WaitOptions::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Wait(ref m) if m.contains("needs a sign-in")),
        "{err:?}"
    );

    let used = upload("up_3", 0, 4, 0, "used", &server);
    Mock::given(method("GET"))
        .and(path("/v1/uploads/up_3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(used))
        .mount(&server)
        .await;
    let err = client(&server)
        .uploads()
        .wait_until_ready("up_3", WaitOptions::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Wait(ref m) if m.contains("already dubbed")),
        "{err:?}"
    );
}

#[tokio::test]
async fn dub_url_chains_the_upload_and_the_dubs_with_suffixed_keys() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/uploads"))
        .and(header("idempotency-key", "order-9:upload"))
        .respond_with(ResponseTemplate::new(200).set_body_json(upload("up_2", 10, 4, 0, "fetching", &server)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/uploads/up_2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(upload("up_2", 10, 4, 0, "uploaded", &server)))
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/v1/dubs")).and(header("idempotency-key", "order-9:dubs"))
        .and(body_json(json!({"upload_id": "up_2", "source_language": "en", "target_languages": ["fr"], "num_speakers": 2})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"dubs": [dub("job_1", "queued")]})))
        .expect(1).mount(&server).await;
    let created = client(&server)
        .with_options(RequestOptions::new().idempotency_key("order-9"))
        .dub_url(
            "https://example.com/a.mp4",
            DubParams::new("en", ["fr"]).num_speakers(2u32),
        )
        .await
        .unwrap();
    assert_eq!(created.dubs[0].id, "job_1");
}

#[tokio::test]
async fn an_idempotency_key_applies_only_to_the_create_call_of_an_upload() {
    let server = MockServer::start().await;
    mount_create(&server).await;
    mount_sign_third(&server).await;
    mount_part(&server, "up_1", 1, b"0123").await;
    mount_part(&server, "up_1", 2, b"4567").await;
    mount_part(&server, "up_1", 3, b"89").await;
    mount_complete(&server).await;
    client(&server)
        .with_options(RequestOptions::new().idempotency_key("mine"))
        .uploads()
        .upload_bytes(DATA, "talk.mp4", UploadOptions::new())
        .await
        .unwrap();
    for r in server.received_requests().await.unwrap() {
        let keyed = r.headers.get("idempotency-key").is_some();
        assert_eq!(
            keyed,
            r.url.path() == "/v1/uploads",
            "{} {}",
            r.method,
            r.url.path()
        );
    }
}

#[tokio::test]
async fn dub_file_sends_the_file_then_starts_the_dubs() {
    let dir = std::env::temp_dir().join(format!("reedapt-dubfile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("talk.mp4");
    std::fs::write(&file, DATA).unwrap();

    let server = MockServer::start().await;
    mount_create(&server).await;
    mount_sign_third(&server).await;
    mount_part(&server, "up_1", 1, b"0123").await;
    mount_part(&server, "up_1", 2, b"4567").await;
    mount_part(&server, "up_1", 3, b"89").await;
    mount_complete(&server).await;
    Mock::given(method("POST"))
        .and(path("/v1/dubs"))
        .and(body_json(
            json!({"upload_id": "up_1", "source_language": "en", "target_languages": ["fr", "es"],
            "instructions": "Warm.", "media_category": "interview"}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"dubs": [dub("job_1", "queued"), dub("job_2", "queued")]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let created = client(&server)
        .dub_file(
            &file,
            DubParams::new("en", ["fr", "es"])
                .instructions("Warm.")
                .media_category("interview"),
        )
        .await
        .unwrap();
    assert_eq!(created.dubs.len(), 2);
    std::fs::remove_dir_all(dir).unwrap();
}
