//! The client side of a server connection: background threads that log in (the Windows account,
//! proved with this PC's device key), keep the connection alive and reconnect, handing messages
//! to the front end through a channel.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::auth::DeviceKey;
use crate::proto::{self, ClientMsg, ServerMsg, UserInfo};
use crate::{Error, Result, auth};

pub type Waker = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug)]
pub struct Config {
    /// `host` or `host:port`.
    pub server: String,
    /// The Windows account, `DOMAIN\user` (see [`identity::windows_account`](crate::identity::windows_account)).
    pub user: String,
    /// The name others see.
    pub display: String,
    /// This PC's device key.
    pub key: Arc<DeviceKey>,
    pub app: String,
    pub machine: String,
}

#[derive(Debug)]
// A handful of events a second, each moved once: boxing every message would cost more than it saves.
#[allow(clippy::large_enum_variant)]
pub enum Event {
    /// Logged in (again, after a reconnect: reopen shared workbooks).
    Connected {
        me: UserInfo,
        users: Vec<UserInfo>,
        server: String,
    },
    Message(ServerMsg),
    /// The connection dropped; the client keeps trying.
    Disconnected(String),
    /// This PC waits for an administrator to approve it (`code`: its device code). The client
    /// tries again quietly every few seconds.
    Pending {
        code: String,
        message: String,
    },
    /// Refused (a revoked device, an incompatible server): the client stopped.
    Failed(String),
}

pub struct Client {
    tx: Sender<ClientMsg>,
    pub events: Receiver<Event>,
    stop: Arc<AtomicBool>,
    pub instance: String,
    pub config: Config,
    online: Arc<AtomicBool>,
}

impl Client {
    pub fn connect(config: Config, waker: Waker) -> Client {
        let (tx, out_rx) = channel::<ClientMsg>();
        let (ev_tx, events) = channel::<Event>();
        let stop = Arc::new(AtomicBool::new(false));
        let online = Arc::new(AtomicBool::new(false));
        let instance = auth::hex(&auth::random_bytes::<8>());
        let (cfg, st, inst, on) = (config.clone(), stop.clone(), instance.clone(), online.clone());
        let spawned = std::thread::Builder::new().name("dexsheets-client".into()).spawn(move || supervise(cfg, inst, out_rx, ev_tx, waker, st, on));
        if let Err(e) = spawned {
            log::error!("could not start the connection thread: {e}");
        }
        Client { tx, events, stop, instance, config, online }
    }

    pub fn send(&self, msg: ClientMsg) {
        let _ = self.tx.send(msg);
    }

    pub fn online(&self) -> bool {
        self.online.load(Ordering::Relaxed)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Seconds between tries while this PC waits for approval.
pub const PENDING_RETRY_S: u64 = 15;

fn address(server: &str) -> String {
    if server.contains(':') { server.to_string() } else { format!("{server}:{}", proto::DEFAULT_PORT) }
}

/// How a login ended.
enum Outcome {
    /// The connection, who we are, everyone, the server's name.
    In(TcpStream, UserInfo, Vec<UserInfo>, String),
    /// This device waits for approval: its code and the server's words.
    Pending(String, String),
}

fn login(cfg: &Config, instance: &str) -> Result<Outcome> {
    let addr = address(&cfg.server);
    let sock = addr.to_socket_addrs()?.next().ok_or_else(|| Error::Protocol(format!("no address for {addr}")))?;
    let mut s = TcpStream::connect_timeout(&sock, Duration::from_secs(4))?;
    s.set_nodelay(true)?;
    s.set_read_timeout(Some(Duration::from_secs(20)))?;
    let public = cfg.key.public_key();
    proto::write_frame(
        &mut s,
        &ClientMsg::Hello {
            protocol: proto::PROTOCOL,
            user: cfg.user.clone(),
            app: cfg.app.clone(),
            machine: cfg.machine.clone(),
            instance: instance.to_string(),
            display: cfg.display.clone(),
            key: auth::hex(&public),
        },
    )?;
    let (nonce, server) = match proto::read_frame::<_, ServerMsg>(&mut s)? {
        ServerMsg::Challenge { nonce, server } => (nonce, server),
        ServerMsg::Error { message, .. } => return Err(Error::Denied(message)),
        other => return Err(Error::Protocol(format!("unexpected {other:?}"))),
    };
    let nonce = auth::unhex(&nonce).ok_or_else(|| Error::Protocol("the server's challenge is not hex".into()))?;
    let signature = cfg.key.sign(&auth::login_message(&nonce, &cfg.user, &public));
    proto::write_frame(&mut s, &ClientMsg::Login { signature: auth::hex(&signature) })?;
    match proto::read_frame::<_, ServerMsg>(&mut s)? {
        ServerMsg::Welcome { me, users, .. } => {
            s.set_read_timeout(None)?;
            Ok(Outcome::In(s, me, users, server))
        }
        ServerMsg::Pending { code, message } => Ok(Outcome::Pending(code, message)),
        ServerMsg::Error { message, .. } => Err(Error::Denied(message)),
        other => Err(Error::Protocol(format!("unexpected {other:?}"))),
    }
}

#[allow(clippy::too_many_arguments)]
fn supervise(
    cfg: Config,
    instance: String,
    out_rx: Receiver<ClientMsg>,
    ev: Sender<Event>,
    waker: Waker,
    stop: Arc<AtomicBool>,
    online: Arc<AtomicBool>,
) {
    let mut backoff = 1u64;
    let mut ever_connected = false;
    while !stop.load(Ordering::Relaxed) {
        match login(&cfg, &instance) {
            Ok(Outcome::Pending(code, message)) => {
                // Not a failure: ask again quietly until an administrator approves this PC.
                let _ = ev.send(Event::Pending { code, message });
                waker();
                for _ in 0..PENDING_RETRY_S * 10 {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                continue;
            }
            Ok(Outcome::In(stream, me, users, server)) => {
                backoff = 1;
                ever_connected = true;
                // Anything queued while offline belonged to the old connection: the front end
                // resends what still matters when it hears `Connected`.
                while out_rx.try_recv().is_ok() {}
                online.store(true, Ordering::Relaxed);
                let _ = ev.send(Event::Connected { me, users, server });
                waker();
                let reason = pump(stream, &out_rx, &ev, &waker, &stop);
                online.store(false, Ordering::Relaxed);
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                let _ = ev.send(Event::Disconnected(reason));
                waker();
            }
            Err(Error::Denied(why)) => {
                let _ = ev.send(Event::Failed(why));
                waker();
                return;
            }
            Err(e) => {
                if !ever_connected {
                    let _ = ev.send(Event::Failed(format!("Cannot reach the server {}: {e}", cfg.server)));
                    waker();
                    return;
                }
                let _ = ev.send(Event::Disconnected(e.to_string()));
                waker();
            }
        }
        for _ in 0..backoff * 10 {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        backoff = (backoff * 2).min(8);
    }
}

/// Runs one connection until it drops; returns why.
fn pump(stream: TcpStream, out_rx: &Receiver<ClientMsg>, ev: &Sender<Event>, waker: &Waker, stop: &Arc<AtomicBool>) -> String {
    let Ok(mut reader) = stream.try_clone() else { return "could not read the connection".into() };
    let alive = Arc::new(AtomicBool::new(true));
    let why = Arc::new(Mutex::new(String::new()));
    let (a, w, e, wk) = (alive.clone(), why.clone(), ev.clone(), waker.clone());
    let spawned = std::thread::Builder::new().name("dexsheets-reader".into()).spawn(move || {
        loop {
            match proto::read_frame::<_, ServerMsg>(&mut reader) {
                Ok(ServerMsg::Pong) => {}
                Ok(m) => {
                    if e.send(Event::Message(m)).is_err() {
                        break;
                    }
                    wk();
                }
                Err(err) => {
                    if let Ok(mut g) = w.lock() {
                        *g = err.to_string();
                    }
                    break;
                }
            }
        }
        a.store(false, Ordering::Relaxed);
    });
    if spawned.is_err() {
        return "could not start the reader".into();
    }
    let mut writer = stream;
    let mut idle = 0u32;
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = writer.shutdown(std::net::Shutdown::Both);
            return "closed".into();
        }
        if !alive.load(Ordering::Relaxed) {
            return why.lock().map(|g| g.clone()).unwrap_or_default();
        }
        match out_rx.recv_timeout(Duration::from_millis(250)) {
            Ok(msg) => {
                idle = 0;
                if let Err(err) = proto::write_frame(&mut writer, &msg) {
                    let _ = writer.shutdown(std::net::Shutdown::Both);
                    return err.to_string();
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                idle += 1;
                // A ping every 10 s finds a dead connection (a cable pulled, a sleeping laptop).
                if idle >= 40 {
                    idle = 0;
                    if let Err(err) = proto::write_frame(&mut writer, &ClientMsg::Ping) {
                        let _ = writer.shutdown(std::net::Shutdown::Both);
                        return err.to_string();
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                // The front end let go of the client: close the socket now, or the reader's
                // clone keeps it open and the server never learns the person is gone.
                let _ = writer.shutdown(std::net::Shutdown::Both);
                return "closed".into();
            }
        }
    }
}
