use async_trait::async_trait;
use serde_json::json;

use super::{HeartbeatKind, Kind, Notification, Notifier, NotifyError};

pub struct DiscordNotifier {
    webhook_url: String,
}

impl DiscordNotifier {
    pub fn new(webhook_url: String) -> Self {
        Self { webhook_url }
    }
}

/// Discord colour conventions: red for bad news, amber for re-open, green for recovery.
fn color(msg: &Notification) -> u32 {
    match msg {
        // crash-red — a new issue or a spike is bad news
        Notification::Issue(n) => match n.kind {
            Kind::NewIssue | Kind::Spike => 0x_E3_40_2D,
            Kind::Reopened => 0x_F0_B4_00, // amber
        },
        Notification::Heartbeat(n) => match n.kind {
            HeartbeatKind::HeartbeatDown => 0x_E3_40_2D,
            HeartbeatKind::HeartbeatRecovered => 0x_2E_CC_71, // green
        },
    }
}

fn fields(msg: &Notification) -> Vec<serde_json::Value> {
    let mut fields = Vec::new();
    match msg {
        Notification::Issue(n) => {
            fields
                .push(json!({"name": "count", "value": n.event_count.to_string(), "inline": true}));
            if let Some(level) = &n.level {
                fields.push(json!({"name": "level", "value": level, "inline": true}));
            }
            if let Some(env) = &n.environment {
                fields.push(json!({"name": "environment", "value": env, "inline": true}));
            }
            if let Some(rel) = &n.release {
                fields.push(json!({"name": "release", "value": rel, "inline": true}));
            }
        }
        Notification::Heartbeat(n) => {
            if let Some(overdue) = n.overdue_seconds {
                fields.push(
                    json!({"name": "overdue", "value": format!("{overdue}s"), "inline": true}),
                );
            }
            if let Some(downtime) = n.downtime_seconds {
                fields.push(
                    json!({"name": "downtime", "value": format!("{downtime}s"), "inline": true}),
                );
            }
        }
    }
    fields
}

#[async_trait]
impl Notifier for DiscordNotifier {
    fn name(&self) -> &'static str {
        "discord"
    }

    async fn send(&self, msg: &Notification) -> Result<(), NotifyError> {
        let body = json!({
            "embeds": [{
                "title": msg.subject(),
                "url": msg.link(),
                "color": color(msg),
                "fields": fields(msg),
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
