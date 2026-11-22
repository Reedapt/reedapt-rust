use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use reedapt::webhook::{self, EventData, WebhookError};
use serde_json::json;
use sha2::Sha256;

const SECRET: &str = "whsec_test";
const BODY: &str = r#"{"id":"evt_1","type":"dub.completed","created_at":"x","data":{"dub":{"id":"job_1","status":"completed","progress":100,"source_language":"en","target_language":"fr","audio":false,"test":true,"created_at":"x"}}}"#;

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn hex_sig(t: u64, body: &str, secret: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("{t}.{body}").as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn sign(body: &str, t: u64, secret: &str) -> String {
    format!("t={t},v1={}", hex_sig(t, body, secret))
}

#[test]
fn a_correctly_signed_delivery_verifies_and_parses() {
    let event = webhook::construct_event(BODY, &sign(BODY, now(), SECRET), SECRET).unwrap();
    assert_eq!(event.id, "evt_1");
    assert_eq!(event.event_type(), "dub.completed");
    match event.data {
        EventData::DubCompleted { dub } => assert_eq!(dub.id, "job_1"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_raw_body_can_be_bytes_or_a_string() {
    let header = sign(BODY, now(), SECRET);
    webhook::verify(BODY.as_bytes(), &header, SECRET).unwrap();
    webhook::verify(String::from(BODY), &header, SECRET).unwrap();
    webhook::verify(Vec::from(BODY.as_bytes()), &header, SECRET).unwrap();
}

/// A signature computed independently with Python's `hmac`, as the Python SDK does:
/// `hmac.new(b"whsec_test", b"1700000000." + body, sha256).hexdigest()`.
#[test]
fn matches_the_reference_signature_from_the_other_sdks() {
    let header = format!("t=1700000000,v1={}", REFERENCE_SIGNATURE);
    webhook::verify_with_tolerance(BODY, &header, SECRET, Duration::from_secs(u64::MAX / 4)).unwrap();
    // And it is too old for the default tolerance.
    assert_eq!(webhook::verify(BODY, &header, SECRET), Err(WebhookError::TooOld));
}

const REFERENCE_SIGNATURE: &str = "48ace5897531831ad83f7de2015fd3ff785d6d4a5a2c4847a2af60a4e7d42915";

#[test]
fn a_changed_body_is_refused() {
    let header = sign(BODY, now(), SECRET);
    assert_eq!(
        webhook::verify(BODY.replace("job_1", "job_2"), &header, SECRET),
        Err(WebhookError::Mismatch)
    );
}

#[test]
fn re_encoded_json_is_refused() {
    let value: serde_json::Value = serde_json::from_str(BODY).unwrap();
    let pretty = serde_json::to_string_pretty(&value).unwrap();
    assert_eq!(
        webhook::verify(pretty, &sign(BODY, now(), SECRET), SECRET),
        Err(WebhookError::Mismatch)
    );
}

#[test]
fn the_wrong_secret_is_refused() {
    let header = sign(BODY, now(), "whsec_other");
    assert_eq!(
        webhook::verify(BODY, &header, SECRET),
        Err(WebhookError::Mismatch)
    );
}

#[test]
fn an_old_signature_is_refused_within_a_configurable_tolerance() {
    let header = sign(BODY, now() - 600, SECRET);
    assert_eq!(webhook::verify(BODY, &header, SECRET), Err(WebhookError::TooOld));
    assert!(webhook::verify(BODY, &header, SECRET)
        .unwrap_err()
        .to_string()
        .contains("too old"));
    webhook::verify_with_tolerance(BODY, &header, SECRET, Duration::from_secs(900)).unwrap();
}

#[test]
fn a_signature_from_the_future_is_refused_too() {
    let header = sign(BODY, now() + 600, SECRET);
    assert_eq!(webhook::verify(BODY, &header, SECRET), Err(WebhookError::TooOld));
}

#[test]
fn a_missing_or_malformed_header_is_refused() {
    assert_eq!(
        webhook::verify(BODY, "", SECRET),
        Err(WebhookError::MissingHeader)
    );
    assert_eq!(
        webhook::verify(BODY, "nonsense", SECRET),
        Err(WebhookError::MalformedHeader)
    );
    assert_eq!(
        webhook::verify(BODY, "t=abc,v1=00", SECRET),
        Err(WebhookError::MalformedHeader)
    );
    assert_eq!(
        webhook::verify(BODY, &format!("t={}", now()), SECRET),
        Err(WebhookError::MalformedHeader)
    );
    assert_eq!(
        webhook::verify(BODY, "v1=00", SECRET),
        Err(WebhookError::MalformedHeader)
    );
}

#[test]
fn garbage_signatures_are_a_mismatch_not_a_panic() {
    let t = now();
    for sig in ["zz", "0", "é", "", &"0".repeat(64)] {
        assert_eq!(
            webhook::verify(BODY, &format!("t={t},v1={sig}"), SECRET),
            Err(WebhookError::Mismatch),
            "{sig:?}"
        );
    }
}

#[test]
fn any_one_of_several_v1_signatures_may_match() {
    let t = now();
    let good = hex_sig(t, BODY, SECRET);
    let header = format!("t={t},v1={},v1={good}", "0".repeat(64));
    webhook::verify(BODY, &header, SECRET).unwrap();
}

#[test]
fn spaces_around_header_items_are_fine() {
    let t = now();
    let header = format!("t={t}, v1={}", hex_sig(t, BODY, SECRET));
    webhook::verify(BODY, &header, SECRET).unwrap();
}

#[test]
fn the_error_converts_into_the_crate_error() {
    let err: reedapt::Error = WebhookError::Mismatch.into();
    assert!(matches!(err, reedapt::Error::Webhook(WebhookError::Mismatch)));
    let from_construct = webhook::construct_event(BODY, "nonsense", SECRET).unwrap_err();
    assert!(matches!(
        from_construct,
        reedapt::Error::Webhook(WebhookError::MalformedHeader)
    ));
}

#[test]
fn a_dub_checked_delivery_parses_to_the_dub_and_its_quality_report() {
    let body = json!({
        "id": "evt_2", "type": "dub.checked", "created_at": "x",
        "data": {
            "dub": {"id": "job_1", "status": "completed", "progress": 100, "source_language": "en",
                    "target_language": "fr", "audio": false, "test": false, "created_at": "x"},
            "quality": {"dub_id": "job_1", "checked": true, "listened": true, "lines": 10, "fixed": 1, "to_review": 0, "flagged": []}
        }
    })
    .to_string();
    let event = webhook::construct_event(&body, &sign(&body, now(), SECRET), SECRET).unwrap();
    match event.data {
        EventData::DubChecked { dub, quality } => {
            assert_eq!(dub.id, "job_1");
            assert_eq!((quality.lines, quality.fixed, quality.to_review), (10, 1, 0));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn failed_batch_and_test_events_parse() {
    let dub = json!({"id": "job_1", "status": "failed", "progress": 10, "source_language": "en",
        "target_language": "fr", "audio": false, "test": false, "created_at": "x",
        "error": {"code": "TRANSCRIPTION_FAILED", "message": "No speech."}});
    let failed =
        json!({"id": "e1", "type": "dub.failed", "created_at": "x", "data": {"dub": dub}}).to_string();
    match webhook::parse_event(failed.as_bytes()).unwrap().data {
        EventData::DubFailed { dub } => assert_eq!(dub.error.unwrap().code, "TRANSCRIPTION_FAILED"),
        other => panic!("{other:?}"),
    }
    let batch = json!({"id": "e2", "type": "batch.completed", "created_at": "x", "data": {"batch": {
        "batch_id": "b1", "status": "done", "video_count": 1, "jobs_total": 1, "jobs_done": 1, "jobs_failed": 0, "dubs": []}}})
    .to_string();
    assert!(matches!(
        webhook::parse_event(batch.as_bytes()).unwrap().data,
        EventData::BatchCompleted { .. }
    ));
    let test = json!({"id": "e3", "type": "webhook.test", "created_at": "x", "data": {}}).to_string();
    assert!(matches!(
        webhook::parse_event(test.as_bytes()).unwrap().data,
        EventData::WebhookTest
    ));
}

#[test]
fn unknown_event_types_are_kept_not_rejected() {
    let body =
        json!({"id": "e9", "type": "dub.something_new", "created_at": "x", "data": {"a": 1}}).to_string();
    let event = webhook::construct_event(&body, &sign(&body, now(), SECRET), SECRET).unwrap();
    assert_eq!(event.event_type(), "dub.something_new");
    match event.data {
        EventData::Unknown { data, .. } => assert_eq!(data["a"], 1),
        other => panic!("{other:?}"),
    }
}
