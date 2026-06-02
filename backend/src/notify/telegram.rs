use async_trait::async_trait;
use serde_json::json;

use super::{Notification, Notifier, NotifyError};

pub struct TelegramNotifier {
    token: String,
    chat_id: String,
    base_url: String,
}

impl TelegramNotifier {
    pub fn new(token: String, chat_id: String) -> Self {
        Self {
            token,
            chat_id,
            base_url: "https://api.telegram.org".into(),
        }
    }

    /// For tests: override the API base URL so a mock server can stand in for telegram.
    #[cfg(test)]
    pub fn with_base_url(mut self, base: String) -> Self {
        self.base_url = base;
        self
    }
}

fn format_body(msg: &Notification) -> String {
    let mut parts = Vec::with_capacity(6);
    parts.push(msg.subject());
    parts.push(format!("count: {}", msg.event_count));
    if let Some(level) = &msg.level {
        parts.push(format!("level: {level}"));
    }
    if let Some(env) = &msg.environment {
        parts.push(format!("env:   {env}"));
    }
    if let Some(rel) = &msg.release {
        parts.push(format!("rel:   {rel}"));
    }
    parts.push(msg.link.clone());
    parts.join("\n")
}

#[async_trait]
impl Notifier for TelegramNotifier {
    fn name(&self) -> &'static str {
        "telegram"
    }

    async fn send(&self, msg: &Notification) -> Result<(), NotifyError> {
        let url = format!("{}/bot{}/sendMessage", self.base_url, self.token);
        let body = json!({
            "chat_id": self.chat_id,
            "text": format_body(msg),
            "disable_web_page_preview": true,
        });
        let resp = reqwest::Client::new().post(url).json(&body).send().await?;
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
