// src/gateway_watch.rs
//
// Emails ALERT_EMAIL when the gateway stops answering, and again when it comes
// back. The gateway mails its own errors, but a gateway that crashes at
// start-up or hangs can't, so the store checks it from outside.
//
// Any HTTP answer, whatever its status, counts as up: only a refused
// connection or a timeout counts as down. After DOWN_AFTER failed checks in a
// row it mails once; when it answers again it mails once more. Off without
// ALERT_EMAIL. The address checked is API0_GATEWAY_WATCH_URL, by default the
// gateway's loopback port.

use graflog::app_log;
use std::time::Duration;

const CHECK_EVERY: Duration = Duration::from_secs(30);
const TIMEOUT: Duration = Duration::from_secs(10);
/// Failed checks in a row before an email: rides out a restart or a deploy.
const DOWN_AFTER: u32 = 4;

pub fn start(store_url: &str) {
    let Some(to) = std::env::var("ALERT_EMAIL").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    else {
        return;
    };
    let target = std::env::var("API0_GATEWAY_WATCH_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:5009/".to_string());
    let mail_url = format!("{}/api/internal/email/send", store_url.trim_end_matches('/'));
    app_log!(info, target = %target, "Watching the gateway");

    tokio::spawn(async move {
        let http = reqwest::Client::builder().timeout(TIMEOUT).build().unwrap_or_default();
        // Let the gateway start alongside the store.
        tokio::time::sleep(Duration::from_secs(60)).await;
        let mut failures = 0u32;
        let mut alerted = false;
        let mut interval = tokio::time::interval(CHECK_EVERY);
        loop {
            interval.tick().await;
            match http.get(&target).send().await {
                Ok(_) => {
                    if alerted {
                        let body = format!("<p>The gateway at {} answers again.</p>", escape(&target));
                        mail(&http, &mail_url, &to, "[api0 gateway] Back up", &body).await;
                        app_log!(info, "Gateway answers again");
                    }
                    failures = 0;
                    alerted = false;
                }
                Err(e) => {
                    failures += 1;
                    let last_error = e.to_string();
                    if failures == DOWN_AFTER && !alerted {
                        alerted = true;
                        app_log!(warn, alert = false, error = %last_error, "Gateway is not answering");
                        let body = format!(
                            "<p>The gateway at {} hasn't answered for {} checks in a row ({} seconds).</p>\
                             <pre style=\"white-space:pre-wrap\">{}</pre>\
                             <p>Look at <code>pm2 status api0-gateway</code> and <code>/var/log/api0/gateway.log</code>.</p>",
                            escape(&target),
                            DOWN_AFTER,
                            DOWN_AFTER as u64 * CHECK_EVERY.as_secs(),
                            escape(&last_error),
                        );
                        mail(&http, &mail_url, &to, "[api0 gateway] Down", &body).await;
                    }
                }
            }
        }
    });
}

async fn mail(http: &reqwest::Client, url: &str, to: &str, subject: &str, html: &str) {
    let secret = std::env::var("API0_INTERNAL_SECRET").unwrap_or_default();
    let result = http
        .post(url)
        .header("X-Internal-Secret", secret)
        .timeout(Duration::from_secs(30))
        .json(&serde_json::json!({
            "to": to,
            "subject": subject,
            "html_body": html,
            "from_name": "api0 alerts",
        }))
        .send()
        .await;
    match result {
        Ok(r) if r.status().is_success() => {}
        Ok(r) => app_log!(warn, alert = false, status = r.status().as_u16(), "Could not email a gateway alert"),
        Err(e) => app_log!(warn, alert = false, error = %e, "Could not email a gateway alert"),
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
