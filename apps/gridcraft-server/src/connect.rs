//! Posts the server's mention and assignment notices to Dexterity Connect (they reach people
//! through Nib; a hub from before `POST /api/bot/notify` gets them as Direct messages), and keeps the
//! "Sheets pulse" card up to date, when an administrator set a hub in the Server Manager
//! (`connect.json`). Runs on its own thread: a hub that is down or slow never holds up the
//! server, and what could not be sent waits in the outbox for the next try. The integration
//! token is sent only to the hub and never logged.

use std::sync::Arc;
use std::time::Duration;

use gridcraft_collab::connect::{self, ConnectConfig, NoticeAnswer, Outgoing};
use gridcraft_collab::server::Server;
use sha2::Digest;

/// How often the outbox is looked at.
const EVERY: Duration = Duration::from_secs(15);
/// The pulse card is sent again at least this often (and whenever it changes).
const PULSE_EVERY_MS: i64 = 5 * 60 * 1000;

/// What became of one post.
#[derive(Debug, PartialEq)]
enum Sent {
    Ok,
    /// Refused for good (nobody to deliver to, a malformed message): not tried again.
    Dropped(String),
    /// Try again later (the hub down, the token refused until an administrator fixes it).
    Later(String),
}

fn client(cfg: &ConnectConfig) -> Result<reqwest::blocking::Client, String> {
    let mut b = reqwest::blocking::Client::builder()
        .user_agent(concat!("DexSheetsServer/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .pool_max_idle_per_host(1);
    if !cfg.fingerprint.is_empty() {
        // The hub's own certificate, recognised by its fingerprint (checked before the token
        // goes out, see `pinned`), instead of by a certificate authority.
        b = b.danger_accept_invalid_certs(true).tls_info(true);
    }
    b.build().map_err(|e| e.to_string())
}

/// With a fingerprint set: the hub's certificate must be the one the administrator pinned. Asked
/// without the token, on the connection the token then goes over.
fn pinned(c: &reqwest::blocking::Client, cfg: &ConnectConfig) -> Result<(), String> {
    if cfg.fingerprint.is_empty() {
        return Ok(());
    }
    let r = c.get(format!("{}/api/bot/widget", cfg.hub)).send().map_err(|e| format!("the hub did not answer: {e}"))?;
    let der = r.extensions().get::<reqwest::tls::TlsInfo>().and_then(|t| t.peer_certificate().map(<[u8]>::to_vec));
    let Some(der) = der else { return Err("the hub did not show a certificate (is the address https://?)".into()) };
    let seen = gridcraft_collab::auth::hex(&sha2::Sha256::digest(&der));
    if seen != cfg.fingerprint {
        return Err(format!("the hub's certificate ({seen}) is not the one set in the Server Manager: nothing was sent"));
    }
    Ok(())
}

/// The hub's status and what it said (`Null` when that is not JSON); `Err` when it did not answer.
fn send(c: &reqwest::blocking::Client, cfg: &ConnectConfig, path: &str, body: &serde_json::Value) -> Result<(u16, serde_json::Value), String> {
    let r = c
        .post(format!("{}{path}", cfg.hub))
        .bearer_auth(cfg.token.trim())
        .header("Content-Type", "application/json")
        .body(serde_json::to_vec(body).unwrap_or_default())
        .send()
        .map_err(|e| format!("the hub did not answer: {}", e.without_url()))?;
    let status = r.status().as_u16();
    // The answer to a notice is a few hundred bytes; anything much larger is not one.
    let said = r.bytes().ok().filter(|b| b.len() <= 256 * 1024).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or(serde_json::Value::Null);
    Ok((status, said))
}

fn post(c: &reqwest::blocking::Client, cfg: &ConnectConfig, path: &str, body: &serde_json::Value) -> Sent {
    match send(c, cfg, path, body) {
        Ok((status, _)) => classify(status),
        Err(why) => Sent::Later(why),
    }
}

/// What the hub's answer to `POST /api/bot/notify` means for the notice; `None` when the hub is
/// from before that request and the notice goes as a Direct message instead.
fn notice_outcome(status: u16, said: &serde_json::Value) -> Option<Sent> {
    match status {
        200..=299 => Some(match connect::notice_answer(said) {
            NoticeAnswer::Delivered => Sent::Ok,
            NoticeAnswer::Unknown => Sent::Dropped("nobody on Connect has that Windows account".into()),
            NoticeAnswer::Failed => Sent::Later("the hub could not deliver the notice yet".into()),
            // 2xx without results: something that is not a Connect hub answered; keep the notice.
            NoticeAnswer::Older => Sent::Later("the hub's answer was not one to a notice".into()),
        }),
        404 if connect::notice_answer(said) == NoticeAnswer::Older => None,
        429 => Some(Sent::Later("the hub asked to wait (too many notices in a minute)".into())),
        _ => Some(classify(status)),
    }
}

/// One notice: through Nib (`/api/bot/notify`), or as a Direct message on an older hub. `older`
/// remembers for the rest of the round that the hub does not know notices.
fn tell(c: &reqwest::blocking::Client, cfg: &ConnectConfig, m: &Outgoing, older: &mut bool) -> Sent {
    if !*older {
        match send(c, cfg, "/api/bot/notify", &connect::notice_body(m)) {
            Err(why) => return Sent::Later(why),
            Ok((status, said)) => match notice_outcome(status, &said) {
                Some(sent) => return sent,
                None => *older = true,
            },
        }
    }
    post(c, cfg, "/api/bot/message", &connect::message_body(m))
}

fn classify(status: u16) -> Sent {
    match status {
        200..=299 => Sent::Ok,
        401 | 403 => Sent::Later(format!("the hub refused the integration token ({status}): set a new one in Server Manager › Dexterity Connect")),
        404 => Sent::Dropped("nobody on Connect has that Windows account".into()),
        400 | 413 => Sent::Dropped(format!("the hub refused the message ({status})")),
        _ => Sent::Later(format!("the hub answered {status}")),
    }
}

/// One round: every notice due, then the pulse card if it changed or is due.
fn round(server: &Server, last_pulse: &mut (Vec<String>, i64)) {
    let cfg = connect::load_config(&server.data_dir());
    if !cfg.ready() {
        return;
    }
    let due = server.connect_due();
    let now = gridcraft_collab::now_ms();
    let lines = server.pulse_lines();
    let pulse_due = lines != last_pulse.0 || now - last_pulse.1 > PULSE_EVERY_MS;
    if due.is_empty() && !pulse_due {
        return;
    }
    let c = match client(&cfg).and_then(|c| pinned(&c, &cfg).map(|()| c)) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Dexterity Connect: {e}");
            for m in &due {
                server.connect_retry(&m.id);
            }
            return;
        }
    };
    let mut older = false;
    for m in due {
        match tell(&c, &cfg, &m, &mut older) {
            Sent::Ok => server.connect_done(&m.id),
            Sent::Dropped(why) => {
                log::warn!("Dexterity Connect: a notice for {} was not delivered: {why}", m.to.join(", "));
                server.connect_done(&m.id);
            }
            Sent::Later(why) => {
                log::warn!("Dexterity Connect: {why}; tried again later");
                server.connect_retry(&m.id);
                // The hub is down or refusing: the rest waits for the next round.
                break;
            }
        }
    }
    if pulse_due {
        match post(&c, &cfg, "/api/bot/widget", &connect::widget_body(&lines)) {
            Sent::Ok => *last_pulse = (lines, now),
            Sent::Dropped(why) | Sent::Later(why) => log::warn!("Dexterity Connect: the Sheets pulse card was not updated: {why}"),
        }
    }
}

/// Posts in the background for as long as the server runs.
pub fn watch(server: Arc<Server>) {
    let mut last_pulse = (Vec::new(), 0i64);
    loop {
        round(&server, &mut last_pulse);
        std::thread::sleep(EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_hub_answers_decides_whether_to_try_again() {
        assert_eq!(classify(200), Sent::Ok);
        assert!(matches!(classify(404), Sent::Dropped(_)), "an account Connect does not know is not retried for ever");
        assert!(matches!(classify(401), Sent::Later(_)), "a refused token is kept until it is fixed");
        assert!(matches!(classify(503), Sent::Later(_)));
        assert!(matches!(classify(413), Sent::Dropped(_)));
    }

    #[test]
    fn a_notice_is_done_kept_or_sent_the_older_way() {
        let json = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap_or(serde_json::Value::Null);
        assert_eq!(notice_outcome(200, &json(r#"{"ok":true,"results":[{"to":"x","status":"delivered","via":"nib"}]}"#)), Some(Sent::Ok));
        assert_eq!(
            notice_outcome(200, &json(r#"{"ok":true,"results":[{"to":"x","status":"duplicate"}]}"#)),
            Some(Sent::Ok),
            "sent twice, shown once"
        );
        assert!(matches!(notice_outcome(200, &json(r#"{"ok":false,"results":[{"to":"x","status":"unknown"}]}"#)), Some(Sent::Dropped(_))));
        assert!(matches!(notice_outcome(200, &json(r#"{"results":[{"status":"failed"}]}"#)), Some(Sent::Later(_))));
        assert!(matches!(notice_outcome(200, &serde_json::Value::Null), Some(Sent::Later(_))), "not a hub's answer: nothing is thrown away");
        assert_eq!(notice_outcome(404, &json(r#"{"ok":false,"error":"not found"}"#)), None, "a hub from before notices: a Direct message instead");
        assert_eq!(notice_outcome(404, &serde_json::Value::Null), None);
        assert!(matches!(notice_outcome(429, &json(r#"{"ok":false,"retryAfterSec":12}"#)), Some(Sent::Later(_))));
        assert!(matches!(notice_outcome(401, &serde_json::Value::Null), Some(Sent::Later(_))), "a refused token is kept until it is fixed");
        assert!(matches!(notice_outcome(500, &serde_json::Value::Null), Some(Sent::Later(_))));
        assert!(matches!(notice_outcome(400, &serde_json::Value::Null), Some(Sent::Dropped(_))));
    }
}
