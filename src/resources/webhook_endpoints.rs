use crate::client::{Client, Req, Retry};
use crate::error::Result;
use crate::resources::seg;
use crate::types::{
    CreateWebhookEndpointParams, WebhookDelivery, WebhookDeliveryList, WebhookEndpoint, WebhookEndpointList,
};

/// Where Reedapt sends events. Check what arrives with [`webhook`](crate::webhook).
#[derive(Debug, Clone)]
pub struct WebhookEndpoints {
    client: Client,
}

impl WebhookEndpoints {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    /// Adds an endpoint. The answer's `secret` is shown only this once.
    ///
    /// Not retried after a network failure: it could add the endpoint twice.
    pub async fn create(&self, params: CreateWebhookEndpointParams) -> Result<WebhookEndpoint> {
        self.client
            .json(Req::post("/webhook_endpoints", Retry::Unsafe).json(&params)?)
            .await
    }

    /// Every endpoint on the account.
    pub async fn list(&self) -> Result<WebhookEndpointList> {
        self.client.json(Req::get("/webhook_endpoints")).await
    }

    /// Removes an endpoint.
    pub async fn delete(&self, endpoint_id: &str) -> Result<()> {
        self.client
            .empty(Req::delete(format!("/webhook_endpoints/{}", seg(endpoint_id))))
            .await
    }

    /// Sends a `webhook.test` event to the endpoint.
    pub async fn send_test(&self, endpoint_id: &str) -> Result<()> {
        self.client
            .empty(Req::post(
                format!("/webhook_endpoints/{}/test", seg(endpoint_id)),
                Retry::Unsafe,
            ))
            .await
    }

    /// Recent deliveries to the endpoint, with what it answered. `limit` is 1 to 100; the
    /// API's default is 50.
    pub async fn deliveries(&self, endpoint_id: &str, limit: Option<u32>) -> Result<WebhookDeliveryList> {
        let mut req = Req::get(format!("/webhook_endpoints/{}/deliveries", seg(endpoint_id)));
        if let Some(limit) = limit {
            req = req.query("limit", limit);
        }
        self.client.json(req).await
    }

    /// Sends a past delivery's event to the endpoint again, now. It keeps its event id,
    /// so a receiver that skips repeats by event id skips it too.
    pub async fn resend_delivery(&self, endpoint_id: &str, delivery_id: &str) -> Result<WebhookDelivery> {
        self.client
            .json(Req::post(
                format!(
                    "/webhook_endpoints/{}/deliveries/{}/resend",
                    seg(endpoint_id),
                    seg(delivery_id)
                ),
                Retry::Unsafe,
            ))
            .await
    }
}
