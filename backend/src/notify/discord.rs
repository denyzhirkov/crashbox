use async_trait::async_trait;
use serde_json::json;

use super::{Kind, Notification, Notifier, NotifyError};

pub struct DiscordNotifier {
    webhook_url: String,
}

impl DiscordNotifier {
    pub fn new(webhook_url: String) -> Self {
        Self { webhook_url }
    }
}

/// Discord colour conventions: red for new issue, amber for re-open / spike.
fn color(kind: Kind) -> u32 {
    match kind {
        // crash-red — a new issue or a spike is bad news
        Kind::NewIssue | Kind::Spike => 0x_E3_40_2D,
        Kind::Reopened => 0x_F0_B4_00, // amber
    }
}

#[async_trait]
impl Notifier for DiscordNotifier {
    fn name(&self) -> &'static str {
        "discord"
    }

    async fn send(&self, msg: &Notification) -> Result<(), NotifyError> {
        let mut fields = Vec::new();
        fields.push(json!({"name": "count", "value": msg.event_count.to_string(), "inline": true}));
        if let Some(level) = &msg.level {
            fields.push(json!({"name": "level", "value": level, "inline": true}));
        }
        if let Some(env) = &msg.environment {
            fields.push(json!({"name": "environment", "value": env, "inline": true}));
        }
        if let Some(rel) = &msg.release {
            fields.push(json!({"name": "release", "value": rel, "inline": true}));
        }

        let body = json!({
            "embeds": [{
                "title": msg.subject(),
                "url": msg.link,
                "color": color(msg.kind),
                "fields": fields,
            }]
        });
        let resp = reqwest::Client::new()
            .post(&self.webhook_url)
            .json(&body)
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
