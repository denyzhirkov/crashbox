//! Generic webhook: POSTs the full `Notification` as JSON to a configured URL.
//!
//! Useful for piping into custom integrations (PagerDuty, Slack via Webhook, internal Kafka
//! relay, …) without us writing a per-vendor adapter for each.

use async_trait::async_trait;

use super::{Notification, NotifyError, Notifier};

pub struct GenericWebhook {
    url: String,
}

impl GenericWebhook {
    pub fn new(url: String) -> Self {
        Self { url }
    }
}

#[async_trait]
impl Notifier for GenericWebhook {
    fn name(&self) -> &'static str {
        "webhook"
    }

    async fn send(&self, msg: &Notification) -> Result<(), NotifyError> {
        let resp = reqwest::Client::new()
            .post(&self.url)
            .json(msg)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(NotifyError::BadStatus {
                status: status.as_u16(),
                body,
            });
        }
        Ok(())
    }
}
