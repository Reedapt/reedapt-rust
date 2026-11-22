use crate::client::{Client, Req};
use crate::error::Result;
use crate::types::{Languages, Usage, UsageReport};

/// The languages dubs can be made in.
#[derive(Debug, Clone)]
pub struct LanguagesApi {
    client: Client,
}

impl LanguagesApi {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    /// The target languages available now.
    pub async fn list(&self) -> Result<Languages> {
        self.client.json(Req::get("/languages")).await
    }
}

/// What the key can do and what it has used.
#[derive(Debug, Clone)]
pub struct UsageApi {
    client: Client,
}

impl UsageApi {
    pub(crate) fn new(client: Client) -> Self {
        Self { client }
    }

    /// The plan, the key's mode and the minutes left.
    pub async fn retrieve(&self) -> Result<Usage> {
        self.client.json(Req::get("/usage")).await
    }

    /// A month (`YYYY-MM`, UTC; this month when `None`) of minutes and dubs, per key and
    /// per language.
    pub async fn report(&self, month: Option<&str>) -> Result<UsageReport> {
        let mut req = Req::get("/usage/report");
        if let Some(month) = month {
            req = req.query("month", month);
        }
        self.client.json(req).await
    }
}
