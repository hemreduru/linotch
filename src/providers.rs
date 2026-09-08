//! Usage sources.
//!
//! Every provider borrows a credential some tool on this machine already holds and
//! asks that vendor's own endpoint. linotch signs in nowhere, stores no secret and
//! never writes a credential file back — a token that has expired is left for its
//! owner to refresh.
//!
//! ## Adding a provider
//!
//! Implement [`Provider`] and add it to [`all`]. `present()` must be cheap and
//! offline: it decides whether a ring is drawn at all, and it runs before any
//! network call.

use serde_json::Value;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// One limit window as the vendor reports it.
#[derive(Clone, Debug)]
pub struct Window {
    pub label: String,
    /// 0.0–1.0 of the allowance consumed.
    pub used: f64,
    /// Absolute reset time, seconds since epoch.
    pub resets_at: Option<i64>,
}

#[derive(Debug)]
pub enum Error {
    /// No usable credential on this machine. Not a failure — the tool is signed out.
    NoCredential,
    /// The stored token's own expiry has passed. Detected locally — no request is
    /// sent, see `Claude::read`.
    Expired,
    /// A credential looked current but the vendor refused it anyway.
    Rejected { code: u16 },
    /// HTTP 429. `retry_after` is the vendor's own Retry-After in seconds — it is
    /// routinely far longer than any backoff we would guess (half an hour, where
    /// our cap was fifteen minutes), and ignoring it just keeps the limit alive.
    RateLimited { retry_after: Option<u64> },
    Other(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoCredential => write!(f, "signed out"),
            Error::Expired => write!(f, "token expired — run `claude` once to refresh it"),
            Error::Rejected { code } => {
                write!(f, "token rejected ({code}) — sign in with the tool itself")
            }
            Error::RateLimited { retry_after: Some(s) } => {
                write!(f, "rate limited — retry {}", until(now_secs() + *s as i64))
            }
            Error::RateLimited { retry_after: None } => write!(f, "rate limited"),
            Error::Other(m) => write!(f, "{m}"),
        }
    }
}

pub trait Provider: Send {
    fn label(&self) -> &'static str;
    /// Name of the embedded brand mark in `assets/` (see `icons`).
    fn asset(&self) -> &'static str;
    /// The mark's colour. Brand colours are what make the rail readable at a
    /// glance without labels; a provider with no colour of its own inherits the
    /// app's accent rather than going grey.
    fn brand(&self) -> (f64, f64, f64) {
        crate::draw::FALLBACK
    }
    /// Offline, cheap. False means the tool is not installed and no ring is drawn.
    fn present(&self) -> bool;
    fn read(&self) -> Result<Vec<Window>, Error>;
}

pub fn all() -> Vec<Box<dyn Provider>> {
    vec![Box::new(Claude), Box::new(Codex)]
}

pub fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

fn read_json(path: &PathBuf) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(|x| x.as_f64())
}

/// RFC3339 or a bare epoch, whichever the vendor felt like sending.
fn parse_reset(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    if let Some(n) = v.as_f64() {
        // Epoch milliseconds are the only way a "seconds" value lands past year 5000.
        return Some(if n > 1e11 { (n / 1000.0) as i64 } else { n as i64 });
    }
    let s = v.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp())
}

fn agent() -> ureq::Agent {
    // A hung request would freeze this provider's thread until the process ends,
    // and the notch would keep showing a stale number with no hint why.
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        // See interpret(): we need the response, not an error, on 4xx.
        .http_status_as_error(false)
        .build()
        .into()
}

/// Turns an HTTP reply into our own error taxonomy.
///
/// Statuses are interpreted here rather than by ureq (`http_status_as_error(false)`)
/// for one reason: a 429's `Retry-After` header only survives if the response object
/// does, and that header is the difference between backing off correctly and
/// hammering a limit until it renews itself.
fn interpret(resp: &mut ureq::http::Response<ureq::Body>) -> Result<Value, Error> {
    let code = resp.status().as_u16();
    match code {
        200..=299 => resp
            .body_mut()
            .read_json()
            .map_err(|e| Error::Other(format!("parse: {e}"))),
        401 | 403 => Err(Error::Rejected { code }),
        429 => Err(Error::RateLimited {
            retry_after: resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse().ok()),
        }),
        _ => Err(Error::Other(format!("HTTP {code}"))),
    }
}

fn now_ms() -> i64 {
    now_secs() * 1000
}

// ------------------------------------------------------------------ Claude

/// Claude Code keeps an OAuth token in `~/.claude/.credentials.json`; the usage
/// endpoint behind `/usage` accepts it directly, so the two never disagree.
pub struct Claude;

impl Claude {
    /// (token, already past its expiry). An expired token is still sent: only the
    /// server decides, and the hint just makes the failure message useful.
    fn credential() -> Option<(String, bool)> {
        for name in [".credentials.json", "credentials.json"] {
            let Some(v) = read_json(&home().join(".claude").join(name)) else {
                continue;
            };
            // Newer Claude Code nests under claudeAiOauth; older wrote it flat.
            let oauth = v.get("claudeAiOauth").unwrap_or(&v);
            if let Some(t) = oauth.get("accessToken").and_then(|x| x.as_str()) {
                if !t.is_empty() {
                    let expired = num(oauth.get("expiresAt"))
                        .map(|ms| (ms as i64) <= now_ms())
                        .unwrap_or(false);
                    return Some((t.to_string(), expired));
                }
            }
        }
        None
    }
}

impl Provider for Claude {
    fn label(&self) -> &'static str {
        "Claude"
    }
    fn asset(&self) -> &'static str {
        "claude"
    }
    fn brand(&self) -> (f64, f64, f64) {
        (0.851, 0.467, 0.341) // #d97757, Anthropic's own
    }

    /// A signed-in Claude Code, not merely a `~/.claude` left behind by one. An
    /// empty dimmed ring for a tool the user does not use is noise, not information.
    fn present(&self) -> bool {
        Self::credential().is_some()
    }

    fn read(&self) -> Result<Vec<Window>, Error> {
        let (token, expired) = Self::credential().ok_or(Error::NoCredential)?;
        if expired {
            // Do not send it. The reply would be a 401, and repeated failed auth is
            // exactly what this endpoint answers with a rate limit — which then also
            // blocks the request that *would* have worked once the token is
            // refreshed. Re-reading the file is free, so this costs nothing to
            // recover from.
            return Err(Error::Expired);
        }
        let mut resp = agent()
            .get("https://api.anthropic.com/api/oauth/usage")
            .header("Authorization", &format!("Bearer {token}"))
            .header("anthropic-beta", "oauth-2025-04-20")
            .call()
            .map_err(|e| Error::Other(e.to_string()))?;
        let v = interpret(&mut resp)?;

        let mut out = Vec::new();
        if let Some(arr) = v.get("limits").and_then(|x| x.as_array()) {
            for l in arr {
                let (Some(kind), Some(pct)) = (
                    l.get("kind").and_then(|x| x.as_str()),
                    num(l.get("percent")),
                ) else {
                    continue;
                };
                out.push(Window {
                    label: claude_label(kind),
                    used: (pct / 100.0).clamp(0.0, 1.0),
                    resets_at: parse_reset(l.get("resets_at")),
                });
            }
        }
        // A window that has just rolled over drops out of `limits` while its named
        // field stays, so the named fields are merged in — but only when `limits`
        // did not already describe the same window.
        for (field, label) in [("five_hour", "Session"), ("seven_day", "Weekly (all)")] {
            let Some(w) = v.get(field) else { continue };
            let Some(u) = num(w.get("utilization")) else {
                continue;
            };
            if out.iter().any(|x| x.label == label) {
                continue;
            }
            out.push(Window {
                label: label.into(),
                used: (u / 100.0).clamp(0.0, 1.0),
                resets_at: parse_reset(w.get("resets_at")),
            });
        }
        Ok(out)
    }
}

fn claude_label(kind: &str) -> String {
    match kind {
        "session" | "five_hour" => "Session".into(),
        "seven_day" | "weekly_all" => "Weekly (all)".into(),
        "seven_day_opus" | "weekly_opus" => "Weekly (Opus)".into(),
        "weekly_scoped" => "Weekly (scoped)".into(),
        other => {
            let mut s = other.replace('_', " ");
            if let Some(c) = s.get_mut(0..1) {
                c.make_ascii_uppercase();
            }
            s
        }
    }
}

// ------------------------------------------------------------------- Codex

/// Codex stores a ChatGPT session in `~/.codex/auth.json`. Read only: the token is
/// never refreshed here, and a 401 simply means Codex will renew it on its own use.
pub struct Codex;

impl Codex {
    fn credential() -> Option<(String, String)> {
        let v = read_json(&home().join(".codex").join("auth.json"))?;
        let t = v.get("tokens")?;
        let access = t.get("access_token")?.as_str()?.trim().to_string();
        let account = t.get("account_id")?.as_str()?.trim().to_string();
        (!access.is_empty() && !account.is_empty()).then_some((access, account))
    }
}

impl Provider for Codex {
    fn label(&self) -> &'static str {
        "Codex"
    }
    fn asset(&self) -> &'static str {
        "openai"
    }
    fn brand(&self) -> (f64, f64, f64) {
        (0.063, 0.639, 0.498) // #10a37f
    }

    fn present(&self) -> bool {
        Self::credential().is_some()
    }

    fn read(&self) -> Result<Vec<Window>, Error> {
        let (access, account) = Self::credential().ok_or(Error::NoCredential)?;
        let mut resp = agent()
            .get("https://chatgpt.com/backend-api/wham/usage")
            .header("Authorization", &format!("Bearer {access}"))
            .header("ChatGPT-Account-Id", &account)
            .header("Accept", "application/json")
            .header(
                "User-Agent",
                concat!("linotch/", env!("CARGO_PKG_VERSION"), " (Linux)"),
            )
            .call()
            .map_err(|e| Error::Other(e.to_string()))?;
        let v = interpret(&mut resp)?;

        let rl = v.get("rate_limit").unwrap_or(&v);
        let now = now_secs();
        let mut out = Vec::new();
        for (label, key) in [("Session", "primary_window"), ("Weekly", "secondary_window")] {
            let Some(w) = rl.get(key) else { continue };
            let Some(pct) = num(w.get("used_percent")) else {
                continue;
            };
            let resets_at = parse_reset(w.get("reset_at"))
                .or_else(|| parse_reset(w.get("resets_at")))
                .or_else(|| num(w.get("reset_after_seconds")).map(|s| now + s as i64));
            out.push(Window {
                label: label.into(),
                used: (pct / 100.0).clamp(0.0, 1.0),
                resets_at,
            });
        }
        Ok(out)
    }
}

// -------------------------------------------------------------------- misc

/// "in 2h 14m" / "in 3d" — reset times are only ever read at a glance.
pub fn until(ts: i64) -> String {
    let d = ts - now_secs();
    if d <= 0 {
        return "now".into();
    }
    let (m, h) = (d / 60, d / 3600);
    if h >= 24 {
        format!("in {}d {}h", h / 24, h % 24)
    } else if h >= 1 {
        format!("in {}h {}m", h, m % 60)
    } else {
        format!("in {}m", m.max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_parsing_accepts_every_shape_the_vendors_send() {
        assert_eq!(parse_reset(Some(&serde_json::json!(1790585719))), Some(1790585719));
        // milliseconds, as Anthropic's older payloads sent them
        assert_eq!(
            parse_reset(Some(&serde_json::json!(1790585719000i64))),
            Some(1790585719)
        );
        assert_eq!(
            parse_reset(Some(&serde_json::json!("2026-09-08T12:00:00Z"))),
            Some(1788868800)
        );
        assert_eq!(parse_reset(Some(&serde_json::json!("nonsense"))), None);
        assert_eq!(parse_reset(None), None);
    }

    #[test]
    fn until_reads_as_english() {
        let n = now_secs();
        assert_eq!(until(n - 10), "now");
        assert_eq!(until(n + 90 * 60), "in 1h 30m");
        assert_eq!(until(n + 30), "in 1m");
        assert!(until(n + 3 * 86400).starts_with("in 3d"));
    }
}
