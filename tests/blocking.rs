#![cfg(feature = "blocking")]

mod common;

use common::{dub, error};
use reedapt::blocking::Client;
use reedapt::types::*;
use reedapt::Error;
use serde_json::json;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The mock server needs a runtime; the blocking client must be used outside one.
fn server() -> (tokio::runtime::Runtime, MockServer, Client) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let server = rt.block_on(MockServer::start());
    let client = Client::builder()
        .api_key("rk_test")
        .base_url(format!("{}/v1", server.uri()))
        .build()
        .unwrap();
    (rt, server, client)
}

#[test]
fn blocking_calls_work_without_async() {
    let (rt, server, client) = server();
    rt.block_on(async {
        Mock::given(method("POST"))
            .and(path("/v1/dubs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"dubs": [dub("job_1", "queued")]})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/dubs/job_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(dub("job_1", "completed")))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/languages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"target_languages": ["fr"]})))
            .mount(&server)
            .await;
    });
    let created = client
        .dubs()
        .create(CreateDubParams::new("up_1", "en", ["fr"]))
        .unwrap();
    assert_eq!(created.dubs[0].id, "job_1");
    let dub = client
        .dubs()
        .wait_until_done("job_1", Default::default())
        .unwrap();
    assert_eq!(dub.status, DubStatus::Completed);
    assert_eq!(client.languages().list().unwrap().target_languages, ["fr"]);
}

#[test]
fn blocking_errors_and_pagination() {
    let (rt, server, client) = server();
    rt.block_on(async {
        for (offset, ids, more) in [("0", vec!["a", "b"], true), ("2", vec!["c"], false)] {
            let data: Vec<_> = ids.iter().map(|i| dub(i, "completed")).collect();
            Mock::given(method("GET"))
                .and(path("/v1/dubs"))
                .and(query_param("offset", offset))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"data": data, "has_more": more})),
                )
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/v1/usage"))
            .respond_with(ResponseTemplate::new(404).set_body_json(error("NOT_FOUND", "No.")))
            .mount(&server)
            .await;
    });
    let ids: Vec<String> = client.dubs().list_all(Some(2)).map(|d| d.unwrap().id).collect();
    assert_eq!(ids, ["a", "b", "c"]);
    assert!(matches!(
        client.usage().retrieve().unwrap_err(),
        Error::Api { status: 404, .. }
    ));
}
