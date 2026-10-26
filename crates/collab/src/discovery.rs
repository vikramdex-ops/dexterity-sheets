//! Finding Dexterity Sheets servers on the LAN: a UDP broadcast question, answered by each
//! server with its name and port.

// Native only (UDP); the wasm clock rule doesn't apply.
#![allow(clippy::disallowed_methods)]

use std::net::UdpSocket;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::proto::DISCOVERY_PORT;

const QUESTION: &[u8] = b"DEXSHEETS?1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Found {
    pub name: String,
    /// `ip:port` to connect to.
    pub address: String,
    pub version: String,
}

/// Asks the LAN for servers and collects answers for `wait`.
pub fn find(wait: Duration) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else { return out };
    if sock.set_broadcast(true).is_err() || sock.set_read_timeout(Some(Duration::from_millis(100))).is_err() {
        return out;
    }
    let _ = sock.send_to(QUESTION, ("255.255.255.255", DISCOVERY_PORT));
    // A server on this machine answers on loopback even where broadcast is filtered.
    let _ = sock.send_to(QUESTION, ("127.0.0.1", DISCOVERY_PORT));
    let start = Instant::now();
    let mut buf = [0u8; 2048];
    while start.elapsed() < wait {
        if let Ok((n, from)) = sock.recv_from(&mut buf)
            && let Some(body) = buf.get(..n)
            && let Ok(mut f) = serde_json::from_slice::<Found>(body)
        {
            // The server reports its port; the address is where the answer came from.
            let port = f.address.rsplit(':').next().unwrap_or("").to_string();
            f.address = format!("{}:{port}", from.ip());
            if !out.iter().any(|o| o.address == f.address || (o.name == f.name && from.ip().is_loopback())) {
                out.push(f);
            }
        }
    }
    out
}

/// Answers discovery questions until the process exits. Runs on its own thread.
pub fn answer(name: String, port: u16, version: String) {
    let Ok(sock) = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)) else {
        log::warn!("discovery: UDP port {DISCOVERY_PORT} is busy; clients must type this server's address");
        return;
    };
    let reply = serde_json::to_vec(&Found { name, address: format!("0.0.0.0:{port}"), version }).unwrap_or_default();
    let mut buf = [0u8; 64];
    loop {
        match sock.recv_from(&mut buf) {
            Ok((n, from)) if buf.get(..n) == Some(QUESTION) => {
                let _ = sock.send_to(&reply, from);
            }
            Ok(_) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}
