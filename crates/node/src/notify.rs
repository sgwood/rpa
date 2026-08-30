use std::env;

use ai_rpa_store::{OutboxItem, Store};
use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::Serialize;
use serde_json::{Value, json};

use crate::config::KEYRING_SERVICE;

#[derive(Clone)]
pub struct FeishuNotifier {
    client: Client,
    webhook: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlushReport {
    pub configured: bool,
    pub attempted: usize,
    pub sent: usize,
    pub failed: usize,
}

impl FeishuNotifier {
    pub fn load() -> Self {
        let webhook = env::var("AI_RPA_FEISHU_WEBHOOK").ok().or_else(|| {
            keyring::Entry::new(KEYRING_SERVICE, "feishu-webhook")
                .ok()
                .and_then(|entry| entry.get_password().ok())
        });
        Self {
            client: Client::new(),
            webhook,
        }
    }

    pub fn configured(&self) -> bool {
        self.webhook.is_some()
    }

    pub async fn send(&self, item: &OutboxItem) -> Result<()> {
        self.send_body(notification_body(item)).await
    }

    pub async fn send_digest(&self, items: &[OutboxItem]) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        self.send_body(digest_body(items)).await
    }

    async fn send_body(&self, body: Value) -> Result<()> {
        let webhook = self
            .webhook
            .as_deref()
            .context("Feishu webhook is not configured")?;
        if !webhook.starts_with("https://") {
            bail!("Feishu webhook must use HTTPS");
        }
        let response = self
            .client
            .post(webhook)
            .json(&body)
            .send()
            .await
            .context("send Feishu notification")?;
        if !response.status().is_success() {
            bail!("Feishu webhook returned HTTP {}", response.status());
        }
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if body.get("code").and_then(Value::as_i64).unwrap_or(0) != 0
            || body.get("StatusCode").and_then(Value::as_i64).unwrap_or(0) != 0
        {
            bail!("Feishu webhook rejected notification");
        }
        Ok(())
    }
}

pub fn configure(webhook: &str) -> Result<()> {
    validate_webhook(webhook)?;
    keyring::Entry::new(KEYRING_SERVICE, "feishu-webhook")?
        .set_password(webhook)
        .context("save Feishu webhook in OS credential store")?;
    Ok(())
}

fn validate_webhook(webhook: &str) -> Result<()> {
    let url = reqwest::Url::parse(webhook).context("Feishu webhook is not a valid URL")?;
    if url.scheme() != "https" {
        bail!("Feishu webhook must use HTTPS");
    }
    let allowed_host = matches!(
        url.host_str(),
        Some("open.feishu.cn") | Some("open.larksuite.com")
    );
    if !allowed_host || !url.path().starts_with("/open-apis/bot/v2/hook/") {
        bail!("only an official Feishu/Lark custom bot webhook is accepted");
    }
    Ok(())
}

fn notification_body(item: &OutboxItem) -> Value {
    let state = item
        .payload
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    let title = item
        .payload
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("未知任务");
    let provider = item
        .payload
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or("UNKNOWN");
    let summary = item
        .payload
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("无摘要");
    let next_step = item
        .payload
        .get("nextStep")
        .and_then(Value::as_str)
        .unwrap_or("查看任务详情");
    json!({
        "msg_type": "interactive",
        "card": {
            "config": {"wide_screen_mode": true},
            "header": {
                "template": match state {
                    "FAILED" => "red",
                    "WAITING_USER" => "orange",
                    "SUCCEEDED" => "green",
                    _ => "blue"
                },
                "title": {"tag": "plain_text", "content": format!("AI 任务状态 · {state}")}
            },
            "elements": [
                {"tag": "markdown", "content": format!("**工具**：{provider}\n**任务**：{title}\n**结论**：{summary}\n**下一步**：{next_step}")},
                {"tag": "note", "elements": [{"tag": "plain_text", "content": format!("事件 ID：{}", item.id)}]}
            ]
        }
    })
}

fn digest_body(items: &[OutboxItem]) -> Value {
    let mut lines = items
        .iter()
        .take(20)
        .map(|item| {
            let provider = item
                .payload
                .get("provider")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN");
            let title = item
                .payload
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("未知任务");
            let duration = item
                .payload
                .get("durationMs")
                .and_then(Value::as_i64)
                .map(|value| format!(" · {} 秒", value / 1000))
                .unwrap_or_default();
            format!("- **{provider}** · {title}{duration}")
        })
        .collect::<Vec<_>>();
    if items.len() > 20 {
        lines.push(format!("- 另有 {} 个成功任务", items.len() - 20));
    }
    json!({
        "msg_type": "interactive",
        "card": {
            "config": {"wide_screen_mode": true},
            "header": {
                "template": "green",
                "title": {"tag": "plain_text", "content": format!("AI 任务完成汇总 · {} 个", items.len())}
            },
            "elements": [
                {"tag": "markdown", "content": lines.join("\n")},
                {"tag": "note", "elements": [{"tag": "plain_text", "content": "仅包含脱敏任务摘要；提示词和源代码不会进入通知。"}]}
            ]
        }
    })
}

pub async fn flush(store: &Store, notifier: &FeishuNotifier) -> Result<FlushReport> {
    if !notifier.configured() {
        return Ok(FlushReport {
            configured: false,
            attempted: 0,
            sent: 0,
            failed: 0,
        });
    }
    let items = store.due_outbox(50)?;
    let mut report = FlushReport {
        configured: true,
        attempted: items.len(),
        sent: 0,
        failed: 0,
    };
    let (digests, immediate): (Vec<_>, Vec<_>) = items
        .into_iter()
        .partition(|item| item.kind == "SUCCEEDED_DIGEST");
    if !digests.is_empty() {
        match notifier.send_digest(&digests).await {
            Ok(()) => {
                for item in &digests {
                    store.mark_outbox_sent(item.id)?;
                    report.sent += 1;
                }
            }
            Err(error) => {
                for item in &digests {
                    store.mark_outbox_retry(item.id, item.attempts + 1, &error.to_string())?;
                    report.failed += 1;
                }
            }
        }
    }
    for item in immediate {
        match notifier.send(&item).await {
            Ok(()) => {
                store.mark_outbox_sent(item.id)?;
                report.sent += 1;
            }
            Err(error) => {
                store.mark_outbox_retry(item.id, item.attempts + 1, &error.to_string())?;
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn accepts_only_official_https_webhooks() {
        assert!(validate_webhook("https://open.feishu.cn/open-apis/bot/v2/hook/abc").is_ok());
        assert!(validate_webhook("http://open.feishu.cn/open-apis/bot/v2/hook/abc").is_err());
        assert!(validate_webhook("https://example.com/open-apis/bot/v2/hook/abc").is_err());
    }

    #[test]
    fn success_digest_groups_multiple_items() {
        let items = ["Codex task", "Claude task"].map(|title| OutboxItem {
            id: Uuid::new_v4(),
            kind: "SUCCEEDED_DIGEST".to_owned(),
            task_id: None,
            payload: json!({"provider":"CODEX","title":title,"durationMs":2000}),
            attempts: 0,
        });
        let body = digest_body(&items);
        assert_eq!(
            body["card"]["header"]["title"]["content"],
            "AI 任务完成汇总 · 2 个"
        );
        assert!(body.to_string().contains("Claude task"));
    }
}
