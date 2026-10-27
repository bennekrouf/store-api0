// src/error_mail.rs
//
// Error alerts by email. Every ERROR-level log line is mailed to ALERT_EMAIL
// through the store's POST /api/internal/email/send. Without ALERT_EMAIL the
// layer does nothing.
//
// Logging is set up here rather than with graflog's init_logging!, which
// installs the global subscriber and leaves no room for another layer. The file
// and console layers are the ones graflog builds.
//
// Throttled, so a failing dependency cannot flood the inbox: one email per
// distinct error every 15 minutes, a follow-up counting the repeats while it
// keeps happening, and at most 30 emails an hour in all. An event opts out with
// an `alert = false` field: the email path's own failures do, so a broken SMTP
// setup cannot mail about itself.
//
// The same file lives in gateway-api0 and store-api0; keep them identical.

use graflog::tracing::field::{Field, Visit};
use graflog::tracing::{Event, Level, Subscriber};
use graflog::tracing_subscriber::layer::Context;
use graflog::tracing_subscriber::Layer;
use graflog::{app_log, LogOption};
use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Write as _};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const REPEAT_WINDOW: Duration = Duration::from_secs(15 * 60);
const HOURLY_CAP: usize = 30;
/// Errors logged before `start`, or faster than they can be mailed, wait here;
/// beyond this many they are dropped rather than held in memory.
const QUEUE: usize = 256;

/// Errors wait here until `start` knows where the store is.
static PENDING: Mutex<Option<mpsc::Receiver<Alert>>> = Mutex::new(None);

#[derive(Clone)]
struct Alert {
    target: String,
    message: String,
    fields: String,
    at: String,
}

impl Alert {
    /// Two errors are "the same" when the same code logs the same text.
    fn key(&self) -> String {
        format!("{} {}", self.target, self.message)
    }
}

// ── Logging set-up ────────────────────────────────────────────────────────────

/// graflog's init_logging!, plus the error-mail layer.
pub fn init_logging(file_path: &str, options: &[LogOption]) {
    use graflog::tracing_subscriber::prelude::*;
    use graflog::tracing_subscriber::{fmt, EnvFilter};

    let mut level = "info".to_string();
    let mut filters = vec![];
    let mut console = true;
    for option in options {
        option.apply(&mut level, &mut filters, &mut console);
    }

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file_path)
        .expect("Failed to open log file");

    let mut filter = EnvFilter::from_default_env()
        .add_directive(level.parse().unwrap_or_else(|e| panic!("Invalid log level '{}': {}", level, e)));
    for directive in filters.iter().flat_map(|f| f.split(',')).map(str::trim).filter(|d| !d.is_empty()) {
        filter = filter.add_directive(
            directive.parse().unwrap_or_else(|e| panic!("Invalid filter directive '{}': {}", directive, e)),
        );
    }

    // A macro, not a closure: each layer sits on a different subscriber type.
    macro_rules! json {
        () => {
            fmt::layer()
                .json()
                .with_current_span(false)
                .with_span_list(false)
                .with_target(false)
                .with_thread_ids(false)
                .with_thread_names(false)
        };
    }

    graflog::tracing_subscriber::registry()
        .with(json!().with_writer(file))
        .with(console.then(|| json!().with_writer(std::io::stdout)))
        .with(layer())
        .with(filter)
        .try_init()
        .ok();
}

// ── The layer ─────────────────────────────────────────────────────────────────

pub struct ErrorMailLayer {
    tx: Option<mpsc::Sender<Alert>>,
}

fn layer() -> ErrorMailLayer {
    if alert_email().is_none() {
        return ErrorMailLayer { tx: None };
    }
    let (tx, rx) = mpsc::channel(QUEUE);
    *PENDING.lock().unwrap() = Some(rx);
    ErrorMailLayer { tx: Some(tx) }
}

fn alert_email() -> Option<String> {
    std::env::var("ALERT_EMAIL").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

impl<S: Subscriber> Layer<S> for ErrorMailLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let Some(tx) = &self.tx else { return };
        if *event.metadata().level() != Level::ERROR {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        if fields.muted {
            return;
        }
        // Full means mail is already backed up; this one is dropped, not waited for.
        let _ = tx.try_send(Alert {
            target: event.metadata().target().to_string(),
            message: fields.message,
            fields: fields.rest,
            at: graflog::chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        });
    }
}

#[derive(Default)]
struct Fields {
    message: String,
    rest: String,
    muted: bool,
}

impl Fields {
    fn put(&mut self, field: &Field, value: String) {
        match field.name() {
            "message" => self.message = value,
            // app_log! adds these to every line; they say nothing here.
            "service" | "component" | "timestamp" => {}
            name => {
                let _ = writeln!(self.rest, "{name}: {value}");
            }
        }
    }
}

impl Visit for Fields {
    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "alert" && !value {
            self.muted = true;
        } else {
            self.put(field, value.to_string());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.to_string());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.put(field, format!("{value:?}"));
    }
}

// ── The mailer ────────────────────────────────────────────────────────────────

/// Starts mailing the errors logged so far and from now on. `service` names
/// this process in the subject; `store_url` is where the store answers.
pub fn start(service: &'static str, store_url: &str) {
    let Some(mut rx) = PENDING.lock().unwrap().take() else { return };
    let Some(to) = alert_email() else { return };
    let mut mailer = Mailer {
        http: reqwest::Client::new(),
        url: format!("{}/api/internal/email/send", store_url.trim_end_matches('/')),
        to,
        service,
        seen: HashMap::new(),
        sent: VecDeque::new(),
        held_back: 0,
    };
    app_log!(info, to = %mailer.to, "Errors will be emailed");
    tokio::spawn(async move {
        // The store may be starting alongside this process: give it a moment
        // before mailing errors logged during start-up.
        tokio::time::sleep(Duration::from_secs(10)).await;
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tokio::select! {
                alert = rx.recv() => match alert {
                    Some(alert) => mailer.on_alert(alert).await,
                    None => break,
                },
                _ = tick.tick() => mailer.flush_repeats().await,
            }
        }
    });
}

struct Seen {
    last_sent: Instant,
    /// Times it happened again since the last email.
    repeats: u32,
    latest: Alert,
}

struct Mailer {
    http: reqwest::Client,
    url: String,
    to: String,
    service: &'static str,
    seen: HashMap<String, Seen>,
    /// When each email of the last hour went out.
    sent: VecDeque<Instant>,
    /// Alerts the hourly cap kept back since the last email.
    held_back: u32,
}

impl Mailer {
    async fn on_alert(&mut self, alert: Alert) {
        if let Some(seen) = self.seen.get_mut(&alert.key()) {
            if seen.last_sent.elapsed() < REPEAT_WINDOW {
                seen.repeats += 1;
                seen.latest = alert;
                return;
            }
        }
        self.send(alert, 0).await;
    }

    /// Mails a count for each error that kept happening after its email.
    async fn flush_repeats(&mut self) {
        let due: Vec<String> = self
            .seen
            .iter()
            .filter(|(_, s)| s.repeats > 0 && s.last_sent.elapsed() >= REPEAT_WINDOW)
            .map(|(k, _)| k.clone())
            .collect();
        for key in due {
            if let Some(seen) = self.seen.remove(&key) {
                self.send(seen.latest, seen.repeats).await;
            }
        }
        // An error quiet for an hour starts afresh next time.
        self.seen.retain(|_, s| s.repeats > 0 || s.last_sent.elapsed() < Duration::from_secs(3600));
    }

    async fn send(&mut self, alert: Alert, repeats: u32) {
        let now = Instant::now();
        while self.sent.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(3600)) {
            self.sent.pop_front();
        }
        if self.sent.len() >= HOURLY_CAP {
            self.held_back += repeats.max(1);
            return;
        }
        self.sent.push_back(now);
        self.seen.insert(alert.key(), Seen { last_sent: now, repeats: 0, latest: alert.clone() });

        let headline: String = alert.message.lines().next().unwrap_or("").chars().take(120).collect();
        let subject = if repeats > 0 {
            format!("[api0 {}] Still failing ({} more): {}", self.service, repeats, headline)
        } else {
            format!("[api0 {}] {}", self.service, headline)
        };
        let mut html = String::new();
        if repeats > 0 {
            let _ = write!(html, "<p>This error happened {repeats} more times since the last email. The latest:</p>");
        }
        let _ = write!(
            html,
            "<p><b>{}</b></p><pre style=\"white-space:pre-wrap\">{}</pre><p style=\"color:#666\">{} · {} · {}</p>",
            escape(&alert.message),
            escape(&alert.fields),
            escape(self.service),
            escape(&alert.target),
            escape(&alert.at),
        );
        if self.held_back > 0 {
            let _ = write!(html, "<p style=\"color:#666\">{} other alerts were held back by the hourly limit; see the log.</p>", self.held_back);
            self.held_back = 0;
        }

        let secret = std::env::var("API0_INTERNAL_SECRET").unwrap_or_default();
        let result = self
            .http
            .post(&self.url)
            .header("X-Internal-Secret", secret)
            .timeout(Duration::from_secs(30))
            .json(&serde_json::json!({
                "to": self.to,
                "subject": subject,
                "html_body": html,
                "from_name": "api0 alerts",
            }))
            .send()
            .await;
        match result {
            Ok(r) if r.status().is_success() => {}
            Ok(r) => app_log!(warn, alert = false, status = r.status().as_u16(), "Could not email an error alert"),
            Err(e) => app_log!(warn, alert = false, error = %e, "Could not email an error alert"),
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use graflog::tracing_subscriber::layer::SubscriberExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A store that answers every request with 200 and counts them.
    async fn fake_store() -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 16 * 1024];
                let _ = sock.read(&mut buf).await;
                seen.fetch_add(1, Ordering::SeqCst);
                let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}").await;
            }
        });
        (url, count)
    }

    fn mailer(url: &str) -> Mailer {
        Mailer {
            http: reqwest::Client::new(),
            url: format!("{url}/api/internal/email/send"),
            to: "ops@example.com".into(),
            service: "test",
            seen: HashMap::new(),
            sent: VecDeque::new(),
            held_back: 0,
        }
    }

    fn alert(message: &str) -> Alert {
        Alert { target: "t".into(), message: message.into(), fields: String::new(), at: String::new() }
    }

    #[test]
    fn only_unmuted_errors_reach_the_mailer() {
        let (tx, mut rx) = mpsc::channel(8);
        let subscriber = graflog::tracing_subscriber::registry().with(ErrorMailLayer { tx: Some(tx) });
        graflog::tracing::subscriber::with_default(subscriber, || {
            app_log!(warn, "just a warning");
            app_log!(error, alert = false, "muted");
            app_log!(error, provider = "x", status = 400, "Provider refused");
        });
        let a = rx.try_recv().unwrap();
        assert_eq!(a.message, "Provider refused");
        assert!(a.fields.contains("provider: x") && a.fields.contains("status: 400"));
        assert!(!a.fields.contains("timestamp"));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn repeats_are_held_then_counted_in_one_follow_up() {
        let (url, count) = fake_store().await;
        let mut m = mailer(&url);
        for _ in 0..5 {
            m.on_alert(alert("same")).await;
        }
        m.on_alert(alert("other")).await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert_eq!(m.seen["t same"].repeats, 4);

        // Nothing is due inside the window; after it, one follow-up.
        m.flush_repeats().await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
        m.seen.get_mut("t same").unwrap().last_sent -= REPEAT_WINDOW;
        m.flush_repeats().await;
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert_eq!(m.seen["t same"].repeats, 0);
    }

    #[tokio::test]
    async fn the_hourly_cap_holds_back_the_rest() {
        let (url, count) = fake_store().await;
        let mut m = mailer(&url);
        for i in 0..HOURLY_CAP + 5 {
            m.on_alert(alert(&format!("error {i}"))).await;
        }
        assert_eq!(count.load(Ordering::SeqCst), HOURLY_CAP);
        assert_eq!(m.held_back, 5);
    }
}
