#![allow(dead_code)]

use reedapt::Client;
use serde_json::{json, Value};
use wiremock::MockServer;

pub fn client(server: &MockServer) -> Client {
    Client::builder()
        .api_key("rk_test_secret")
        .base_url(format!("{}/v1/", server.uri()))
        .build()
        .unwrap()
}

pub fn dub(id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "object": "dub",
        "status": status,
        "stage": null,
        "progress": 0,
        "source_language": "en",
        "target_language": "fr",
        "audio": false,
        "test": true,
        "created_at": "2026-11-22T10:00:00Z",
        "metadata": {"order_id": "1042"}
    })
}

pub fn error(code: &str, message: &str) -> Value {
    json!({"error": {"code": code, "message": message, "request_id": "req_123"}})
}

pub fn upload(
    id: &str,
    size: u64,
    part_size: u64,
    part_count: u32,
    status: &str,
    server: &MockServer,
) -> Value {
    let parts: Vec<Value> = (1..=part_count.min(2))
        .map(|n| json!({"part_number": n, "url": format!("{}/storage/{id}/{n}", server.uri())}))
        .collect();
    json!({
        "id": id, "object": "upload", "filename": "talk.mp4", "size": size,
        "status": status, "part_size": part_size, "part_count": part_count, "parts": parts
    })
}
