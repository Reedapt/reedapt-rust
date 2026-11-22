mod common;

use std::time::{Duration, Instant};

use common::{client, dub, error};
use futures_util::TryStreamExt;
use reedapt::types::*;
use reedapt::{Client, Error, RequestOptions};
use serde_json::json;
use wiremock::matchers::{body_json, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn requests_carry_the_key_user_agent_and_the_base_url() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/languages"))
        .and(header("authorization", "Bearer rk_test_secret"))
        .and(header(
            "user-agent",
            format!("reedapt-rust/{}", reedapt::VERSION).as_str(),
        ))
        .and(header("accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"target_languages": ["fr", "es"]})))
        .expect(1)
        .mount(&server)
        .await;
    let languages = client(&server).languages().list().await.unwrap();
    assert_eq!(languages.target_languages, ["fr", "es"]);
}

#[test]
fn a_client_needs_a_key_and_never_prints_it() {
    // SAFETY of env use: this test doesn't run other threads reading the variable.
    std::env::remove_var("REEDAPT_API_KEY");
    assert!(matches!(Client::builder().build(), Err(Error::Config(_))));
    let c = Client::new("rk_live_hunter2");
    assert!(!format!("{c:?}").contains("hunter2"));
    assert_eq!(c.base_url(), "https://api.reedapt.com/v1");
}

#[tokio::test]
async fn dubs_create_sends_the_params_and_an_idempotency_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/dubs"))
        .and(header_exists("idempotency-key"))
        .and(body_json(json!({
            "upload_id": "upl_1",
            "source_language": "en",
            "target_languages": ["fr", "xx"],
            "voice_cloning": true,
            "instructions": "Keep names.",
            "metadata": {"order_id": "1042"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dubs": [dub("job_1", "queued")],
            "refused": [{"target_language": "xx", "code": "LANGUAGE_NOT_SUPPORTED", "message": "No xx."}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let params = CreateDubParams::new("upl_1", "en", ["fr", "xx"])
        .voice_cloning(true)
        .instructions("Keep names.")
        .metadata("order_id", "1042");
    let created = client(&server).dubs().create(params).await.unwrap();
    assert_eq!(created.dubs[0].id, "job_1");
    assert_eq!(created.dubs[0].status, DubStatus::Queued);
    assert_eq!(created.dubs[0].metadata["order_id"], "1042");
    assert_eq!(created.refused[0].code, "LANGUAGE_NOT_SUPPORTED");
}

#[tokio::test]
async fn a_caller_idempotency_key_is_used_as_given() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/dubs"))
        .and(header("idempotency-key", "order-1042"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"dubs": []})))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .with_options(RequestOptions::new().idempotency_key("order-1042"))
        .dubs()
        .create(CreateDubParams::new("upl_1", "en", ["fr"]))
        .await
        .unwrap();
}

#[tokio::test]
async fn reads_dubs() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/dubs/job_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(dub("job_1", "processing")))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/dubs"))
        .and(query_param("limit", "5"))
        .and(query_param("offset", "10"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": [dub("job_2", "completed")], "has_more": true})),
        )
        .mount(&server)
        .await;
    let c = client(&server);
    assert_eq!(
        c.dubs().retrieve("job_1").await.unwrap().status,
        DubStatus::Processing
    );
    let page = c
        .dubs()
        .list(ListDubsParams::new().limit(5).offset(10))
        .await
        .unwrap();
    assert!(page.has_more);
    assert_eq!(page.data[0].id, "job_2");
}

#[tokio::test]
async fn ids_are_escaped_in_paths() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/dubs/a%2Fb%20c"))
        .respond_with(ResponseTemplate::new(200).set_body_json(dub("x", "queued")))
        .expect(1)
        .mount(&server)
        .await;
    client(&server).dubs().retrieve("a/b c").await.unwrap();
}

#[tokio::test]
async fn cancel_files_transcript_quality() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/dubs/job_1/cancel"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/dubs/job_1/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "files": [{"kind": "dub", "filename": "a.mp4", "url": "https://x/a.mp4"},
                      {"kind": "something_new", "filename": "b", "url": "https://x/b"}],
            "expires_at": "2026-11-22T11:00:00Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/v1/dubs/job_1/transcript"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dub_id": "job_1", "source_language": "en", "target_language": "fr",
            "lines": [{"start_ms": 0, "end_ms": 900, "speaker": "SPEAKER_00", "original_text": "Hello", "text": "Bonjour"}]
        }))).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/dubs/job_1/quality"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dub_id": "job_1", "checked": true, "listened": true, "lines": 10, "fixed": 1, "to_review": 1,
            "flagged": [{"line": 3, "start_ms": 5000, "checks": ["fit"], "fixed": "shortened"},
                        {"line": 4, "start_ms": 6000, "checks": ["fit"], "fixed": null}]
        })))
        .mount(&server)
        .await;
    let c = client(&server);
    c.dubs().cancel("job_1").await.unwrap();
    let files = c.dubs().files("job_1").await.unwrap();
    assert_eq!(files.files[0].kind, FileKind::Dub);
    assert_eq!(files.files[1].kind, FileKind::Unknown);
    let t = c.dubs().transcript("job_1").await.unwrap();
    assert_eq!(t.lines[0].text, "Bonjour");
    assert_eq!(t.lines[0].speaker.as_deref(), Some("SPEAKER_00"));
    let q = c.dubs().quality("job_1").await.unwrap();
    assert_eq!(q.flagged[0].fixed, Some(QualityFix::Shortened));
    assert_eq!(q.flagged[1].fixed, None);
}

#[tokio::test]
async fn subtitles_come_back_as_text() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/dubs/job_1/subtitles"))
        .and(query_param("format", "vtt"))
        .and(query_param("language", "source"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("WEBVTT\n\n00:00.000 --> 00:01.000\nHello\n"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let text = client(&server)
        .dubs()
        .subtitles(
            "job_1",
            SubtitlesParams::new()
                .format(SubtitleFormat::Vtt)
                .language(SubtitleLanguage::Source),
        )
        .await
        .unwrap();
    assert!(text.starts_with("WEBVTT"));
}

#[tokio::test]
async fn requests_runs_and_lines() {
    let server = MockServer::start().await;
    let run = json!({
        "id": "run_1", "dub_id": "job_1", "status": "needs_input", "request": "calmer", "reply": "I can.",
        "proposal": {"kind": "restyle", "summary": "Soften line 3", "lines": 1},
        "options": [{"id": "apply", "label": "Apply"}, {"id": "decline", "label": "Decline"}],
        "created_at": "2026-11-22T10:00:00Z"
    });
    Mock::given(method("POST"))
        .and(path("/v1/dubs/job_1/requests"))
        .and(body_json(
            json!({"text": "Make line 3 calmer", "autonomy": "ask"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(&run))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/runs/run_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&run))
        .mount(&server)
        .await;
    let mut applied = run.clone();
    applied["status"] = json!("applied");
    applied["applied_lines"] = json!(1);
    Mock::given(method("POST"))
        .and(path("/v1/runs/run_1/answer"))
        .and(body_json(json!({"choice": "apply"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(&applied))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/v1/dubs/job_1/lines"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "dub_id": "job_1",
            "lines": [{"line": 1, "start_ms": 0, "end_ms": 500, "original_text": "Hi", "text": "Salut", "delivery_style": "warm"}]
        }))).mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/dubs/job_1/lines/edit"))
        .and(body_json(json!({"lines": [{"line": 3, "text": "Bienvenue.", "delivery_style": null, "voice_instructions": "softly"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"dub_id": "job_1", "lines": [3], "status": "revoicing"})))
        .expect(1).mount(&server).await;

    let c = client(&server);
    let r = c
        .dubs()
        .requests()
        .create("job_1", CreateRunParams::new("Make line 3 calmer"))
        .await
        .unwrap();
    assert_eq!(r.status, RunStatus::NeedsInput);
    assert_eq!(r.proposal.as_ref().unwrap().lines, 1);
    assert_eq!(r.options.as_ref().unwrap()[0].id, RunChoice::Apply);
    assert_eq!(c.runs().retrieve("run_1").await.unwrap().id, "run_1");
    let done = c.runs().answer("run_1", RunChoice::Apply).await.unwrap();
    assert_eq!((done.status, done.applied_lines), (RunStatus::Applied, Some(1)));
    let lines = c.dubs().lines().list("job_1").await.unwrap();
    assert_eq!(lines.lines[0].delivery_style.as_deref(), Some("warm"));
    let edited = c
        .dubs()
        .lines()
        .edit(
            "job_1",
            vec![LineEdit::new(3)
                .text("Bienvenue.")
                .clear_delivery_style()
                .voice_instructions("softly")],
        )
        .await
        .unwrap();
    assert_eq!(edited.lines, [3]);
}

#[tokio::test]
async fn languages_usage_and_report() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/usage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "plan": "pro", "key_mode": "test", "minutes_remaining": 12.5, "unlimited": false, "test_preview_seconds": 60
        }))).mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/usage/report")).and(query_param("month", "2026-10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "month": "2026-10", "period_start": "2026-10-01T00:00:00Z", "period_end": "2026-11-01T00:00:00Z",
            "minutes": 10.5, "minutes_in_progress": 1.0, "dubs": {"completed": 3, "in_progress": 1, "failed": 0},
            "keys": [{"key_id": "k1", "mode": "live", "minutes": 10.5, "minutes_in_progress": 1.0,
                      "dubs": {"completed": 3, "in_progress": 1, "failed": 0}}],
            "languages": [{"target_language": "fr", "minutes": 10.5, "minutes_in_progress": 1.0,
                           "dubs": {"completed": 3, "in_progress": 1, "failed": 0}}]
        }))).expect(1).mount(&server).await;
    let c = client(&server);
    let usage = c.usage().retrieve().await.unwrap();
    assert_eq!(
        (usage.key_mode, usage.minutes_remaining),
        (KeyMode::Test, Some(12.5))
    );
    let report = c.usage().report(Some("2026-10")).await.unwrap();
    assert_eq!(report.totals.dubs.completed, 3);
    assert_eq!(report.keys[0].mode, KeyMode::Live);
    assert_eq!(report.keys[0].usage.minutes, 10.5);
    assert_eq!(report.languages[0].target_language.as_deref(), Some("fr"));
}

#[tokio::test]
async fn webhook_endpoints() {
    let server = MockServer::start().await;
    let ep = json!({"id": "we_1", "url": "https://x/h", "events": ["dub.completed"], "created_at": "2026-11-22T10:00:00Z", "secret": "whsec_abc"});
    Mock::given(method("POST"))
        .and(path("/v1/webhook_endpoints"))
        .and(body_json(json!({"url": "https://x/h", "description": "prod"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(&ep))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/webhook_endpoints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [ep]})))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/webhook_endpoints/we_1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/webhook_endpoints/we_1/test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"queued": true})))
        .expect(1)
        .mount(&server)
        .await;
    let delivery = json!({"id": "dl_1", "event_id": "evt_1", "event_type": "dub.completed", "status": "failed",
        "attempts": 3, "last_status_code": 500, "created_at": "2026-11-22T10:00:00Z"});
    Mock::given(method("GET"))
        .and(path("/v1/webhook_endpoints/we_1/deliveries"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [delivery]})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/webhook_endpoints/we_1/deliveries/dl_1/resend"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&delivery))
        .expect(1)
        .mount(&server)
        .await;

    let c = client(&server);
    let w = c.webhook_endpoints();
    let created = w
        .create(CreateWebhookEndpointParams::new("https://x/h").description("prod"))
        .await
        .unwrap();
    assert_eq!(created.secret.as_deref(), Some("whsec_abc"));
    assert_eq!(w.list().await.unwrap().data.len(), 1);
    w.send_test("we_1").await.unwrap();
    let d = w.deliveries("we_1", Some(10)).await.unwrap();
    assert_eq!(d.data[0].last_status_code, Some(500));
    assert_eq!(w.resend_delivery("we_1", "dl_1").await.unwrap().event_id, "evt_1");
    w.delete("we_1").await.unwrap();
}

// ------------------------------------------------------------------ errors

#[tokio::test]
async fn api_errors_carry_status_code_and_request_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/dubs/nope"))
        .respond_with(ResponseTemplate::new(404).set_body_json(error("NOT_FOUND", "No such dub.")))
        .expect(1) // 404 is never retried
        .mount(&server)
        .await;
    let err = client(&server).dubs().retrieve("nope").await.unwrap_err();
    assert!(err.is_not_found());
    assert_eq!(err.status(), Some(404));
    assert_eq!(err.code(), Some("NOT_FOUND"));
    assert_eq!(err.request_id(), Some("req_123"));
    assert_eq!(err.to_string(), "NOT_FOUND: No such dub.");
    assert!(matches!(err, Error::Api { status: 404, .. }));
}

#[tokio::test]
async fn field_problems_are_listed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/dubs"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({"error": {
            "code": "INVALID_REQUEST", "message": "Bad.", "request_id": "r",
            "fields": [{"field": "target_languages", "message": "Pick at least one."}]
        }})))
        .mount(&server)
        .await;
    let err = client(&server)
        .dubs()
        .create(CreateDubParams::new("u", "en", Vec::<String>::new()))
        .await
        .unwrap_err();
    match err {
        Error::Api {
            status: 422, fields, ..
        } => assert_eq!(fields[0].field, "target_languages"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn an_error_without_a_body_still_has_a_code() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/usage"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("request-id", "req_hdr")
                .set_body_string("nope"),
        )
        .mount(&server)
        .await;
    let err = client(&server).usage().retrieve().await.unwrap_err();
    assert!(err.is_authentication());
    assert_eq!(err.code(), Some("HTTP_401"));
    assert_eq!(err.request_id(), Some("req_hdr"));
}

#[tokio::test]
async fn per_minute_rate_limits_are_retried_and_honour_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/languages"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "1")
                .set_body_json(error("RATE_LIMITED", "Slow down.")),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/languages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"target_languages": ["fr"]})))
        .expect(1)
        .mount(&server)
        .await;
    let started = Instant::now();
    client(&server).languages().list().await.unwrap();
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "waited {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn long_lasting_rate_limits_are_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/dubs"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "30")
                .set_body_json(error("TOO_MANY_DUBS_RUNNING", "Wait for some to finish.")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let err = client(&server)
        .dubs()
        .create(CreateDubParams::new("u", "en", ["fr"]))
        .await
        .unwrap_err();
    match err {
        Error::RateLimited {
            code,
            retry_after,
            request_id,
            ..
        } => {
            assert_eq!(code, "TOO_MANY_DUBS_RUNNING");
            assert_eq!(retry_after, Some(Duration::from_secs(30)));
            assert_eq!(request_id.as_deref(), Some("req_123"));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn server_errors_are_retried_then_surface() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/usage"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("retry-after", "0")
                .set_body_json(error("UNAVAILABLE", "Busy.")),
        )
        .expect(3) // the first try and two retries
        .mount(&server)
        .await;
    let err = client(&server).usage().retrieve().await.unwrap_err();
    assert_eq!(err.status(), Some(503));
}

#[tokio::test]
async fn max_retries_can_be_lowered_per_call() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/usage"))
        .respond_with(ResponseTemplate::new(500).set_body_json(error("INTERNAL", "Oops.")))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .with_options(RequestOptions::new().max_retries(0))
        .usage()
        .retrieve()
        .await
        .unwrap_err();
}

#[tokio::test]
async fn a_500_on_a_non_idempotent_post_is_not_retried_but_a_503_is() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/webhook_endpoints"))
        .respond_with(ResponseTemplate::new(500).set_body_json(error("INTERNAL", "Oops.")))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .webhook_endpoints()
        .create(CreateWebhookEndpointParams::new("https://x"))
        .await
        .unwrap_err();

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/webhook_endpoints"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("retry-after", "0")
                .set_body_json(error("UNAVAILABLE", "Busy.")),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/webhook_endpoints"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "we_1", "url": "https://x", "events": [], "created_at": "t"})))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .webhook_endpoints()
        .create(CreateWebhookEndpointParams::new("https://x"))
        .await
        .unwrap();
}

#[tokio::test]
async fn creating_dubs_retries_a_500_because_it_is_idempotent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/dubs"))
        .respond_with(ResponseTemplate::new(500).set_body_json(error("INTERNAL", "Oops.")))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/dubs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"dubs": [dub("job_1", "queued")]})))
        .expect(1)
        .mount(&server)
        .await;
    // The retry reuses one key, so the API can recognise it.
    let created = client(&server)
        .dubs()
        .create(CreateDubParams::new("u", "en", ["fr"]))
        .await
        .unwrap();
    assert_eq!(created.dubs.len(), 1);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].headers.get("idempotency-key").is_some());
    assert_eq!(
        requests[0].headers.get("idempotency-key"),
        requests[1].headers.get("idempotency-key")
    );
}

#[tokio::test]
async fn idempotency_in_progress_conflicts_are_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/dubs"))
        .respond_with(
            ResponseTemplate::new(409)
                .insert_header("retry-after", "0")
                .set_body_json(error("IDEMPOTENCY_IN_PROGRESS", "Still working.")),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/dubs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"dubs": []})))
        .expect(1)
        .mount(&server)
        .await;
    client(&server)
        .dubs()
        .create(CreateDubParams::new("u", "en", ["fr"]))
        .await
        .unwrap();
}

#[tokio::test]
async fn an_unreadable_answer_is_a_decode_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/usage"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
        .mount(&server)
        .await;
    let err = client(&server).usage().retrieve().await.unwrap_err();
    assert!(matches!(err, Error::Decode(_)), "{err:?}");
}

#[tokio::test]
async fn a_dead_server_is_a_transport_error_after_retries() {
    // Nothing listens on a port that was just released.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let c = Client::builder()
        .api_key("k")
        .base_url(format!("http://127.0.0.1:{port}/v1"))
        .max_retries(1)
        .build()
        .unwrap();
    let err = c.languages().list().await.unwrap_err();
    assert!(matches!(err, Error::Transport(_)), "{err:?}");
    assert_eq!(err.status(), None);
}

#[tokio::test]
async fn a_slow_answer_times_out() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/usage"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(500))
                .set_body_json(json!({})),
        )
        .mount(&server)
        .await;
    let err = client(&server)
        .with_options(
            RequestOptions::new()
                .timeout(Duration::from_millis(50))
                .max_retries(0),
        )
        .usage()
        .retrieve()
        .await
        .unwrap_err();
    assert!(err.is_timeout(), "{err:?}");
}

#[tokio::test]
async fn a_custom_http_client_is_used() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/languages"))
        .and(header("x-proxy", "yes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"target_languages": []})))
        .expect(1)
        .mount(&server)
        .await;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-proxy", "yes".parse().unwrap());
    let http = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap();
    Client::builder()
        .api_key("k")
        .base_url(format!("{}/v1", server.uri()))
        .http_client(http)
        .build()
        .unwrap()
        .languages()
        .list()
        .await
        .unwrap();
}

#[tokio::test]
async fn unknown_fields_and_values_are_tolerated() {
    let server = MockServer::start().await;
    let mut d = dub("job_1", "a_status_from_the_future");
    d["brand_new_field"] = json!({"nested": true});
    Mock::given(method("GET"))
        .and(path("/v1/dubs/job_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(d))
        .mount(&server)
        .await;
    let dub = client(&server).dubs().retrieve("job_1").await.unwrap();
    assert_eq!(dub.status, DubStatus::Unknown);
    assert!(!dub.status.is_finished());
}

// -------------------------------------------------------------- pagination

#[tokio::test]
async fn list_all_streams_every_page_and_skips_repeats() {
    let server = MockServer::start().await;
    let page = |offset: &str, ids: &[&str], more: bool| {
        let data: Vec<_> = ids.iter().map(|i| dub(i, "completed")).collect();
        (offset.to_string(), json!({"data": data, "has_more": more}))
    };
    // A dub started while paging shifts page 2 by one, so job_2 shows up twice.
    for (offset, body) in [
        page("0", &["job_1", "job_2"], true),
        page("2", &["job_2", "job_3"], true),
        page("4", &["job_4"], false),
    ] {
        Mock::given(method("GET"))
            .and(path("/v1/dubs"))
            .and(query_param("limit", "2"))
            .and(query_param("offset", offset.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
    }
    let ids: Vec<String> = client(&server)
        .dubs()
        .list_all(Some(2))
        .map_ok(|d| d.id)
        .try_collect()
        .await
        .unwrap();
    assert_eq!(ids, ["job_1", "job_2", "job_3", "job_4"]);
}

#[tokio::test]
async fn list_all_yields_the_error_and_stops() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/dubs"))
        .respond_with(ResponseTemplate::new(401).set_body_json(error("UNAUTHORIZED", "Bad key.")))
        .expect(1)
        .mount(&server)
        .await;
    let mut stream = Box::pin(client(&server).dubs().list_all(None));
    let first = futures_util::StreamExt::next(&mut stream).await.unwrap();
    assert!(first.unwrap_err().is_authentication());
    assert!(futures_util::StreamExt::next(&mut stream).await.is_none());
}

// ------------------------------------------------------------------ waiting

#[tokio::test]
async fn wait_until_done_polls_until_the_dub_finishes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/dubs/job_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(dub("job_1", "processing")))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/dubs/job_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(dub("job_1", "completed")))
        .mount(&server)
        .await;
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = seen.clone();
    let dub = client(&server)
        .dubs()
        .wait_until_done(
            "job_1",
            WaitForDubOptions::new()
                .poll_interval(Duration::from_millis(10))
                .on_update(move |_| {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }),
        )
        .await
        .unwrap();
    assert_eq!(dub.status, DubStatus::Completed);
    assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[tokio::test]
async fn wait_until_done_gives_up() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/dubs/job_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(dub("job_1", "processing")))
        .mount(&server)
        .await;
    let err = client(&server)
        .dubs()
        .wait_until_done(
            "job_1",
            WaitForDubOptions::new()
                .poll_interval(Duration::from_millis(10))
                .timeout(Duration::from_millis(30)),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Wait(_)), "{err:?}");
}

use reedapt::resources::WaitForDubOptions;
