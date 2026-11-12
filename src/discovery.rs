//! Find projectors on the local network.
//!
//! Two passes run together:
//! 1. PJLink Class 2 search: broadcast `%2SRCH` over UDP 4352 and collect `%2ACKN` replies.
//! 2. Subnet sweep: try TCP 4352 (PJLink) and 1024 (Panasonic) on every host of
//!    each local /24, which also finds Class 1 projectors that ignore the broadcast.

use crate::model::Protocol;
use crate::net::LineConn;
use crate::pjlink::parse_reply;
use crate::pjlink::Reply;
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PROBE_CONNECT: Duration = Duration::from_millis(450);
const PROBE_IO: Duration = Duration::from_millis(1500);
const WORKERS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub ip: Ipv4Addr,
    pub port: u16,
    pub protocol: Protocol,
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    pub needs_password: bool,
}

impl Found {
    pub fn title(&self) -> String {
        if !self.name.trim().is_empty() {
            self.name.trim().to_string()
        } else if !self.model.trim().is_empty() {
            format!("{} {}", self.manufacturer.trim(), self.model.trim())
                .trim()
                .to_string()
        } else {
            format!("Projector {}", self.ip)
        }
    }
}

#[derive(Clone)]
pub struct Scan {
    pub results: Arc<Mutex<Vec<Found>>>,
    pub done: Arc<AtomicUsize>,
    pub total: Arc<AtomicUsize>,
    pub running: Arc<AtomicBool>,
    pub subnets: Arc<Mutex<Vec<String>>>,
}

impl Scan {
    pub fn progress(&self) -> f32 {
        let total = self.total.load(Ordering::Relaxed).max(1);
        self.done.load(Ordering::Relaxed) as f32 / total as f32
    }
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

/// Parse "192.168.10", "192.168.10.0/24" or "192.168.10.57" into a /24 prefix.
pub fn parse_prefix(text: &str) -> Option<[u8; 3]> {
    let t = text.trim().split('/').next()?.trim().trim_end_matches('.');
    let parts: Vec<u8> = t
        .split('.')
        .map(|p| p.trim().parse::<u8>())
        .collect::<Result<_, _>>()
        .ok()?;
    if parts.len() == 3 || parts.len() == 4 {
        Some([parts[0], parts[1], parts[2]])
    } else {
        None
    }
}

/// /24 prefixes of this machine's non-loopback IPv4 interfaces.
pub fn local_prefixes() -> Vec<[u8; 3]> {
    let mut set = BTreeSet::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if iface.is_loopback() {
                continue;
            }
            if let IpAddr::V4(ip) = iface.ip() {
                let o = ip.octets();
                if o[0] == 169 && o[1] == 254 {
                    continue; // link-local: nothing useful to sweep
                }
                set.insert([o[0], o[1], o[2]]);
            }
        }
    }
    set.into_iter().collect()
}

fn broadcast_addrs() -> Vec<Ipv4Addr> {
    let mut out = vec![Ipv4Addr::BROADCAST];
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if let if_addrs::IfAddr::V4(v4) = iface.addr {
                if let Some(b) = v4.broadcast {
                    if !out.contains(&b) {
                        out.push(b);
                    }
                }
            }
        }
    }
    out
}

/// PJLink Class 2 UDP search. Returns the addresses that answered.
fn udp_search(wait: Duration) -> Vec<Ipv4Addr> {
    // Replies normally come back to the sending port, but some projectors
    // answer to 4352, so prefer binding that when it is free.
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 4352))
        .or_else(|_| UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)));
    let Ok(socket) = socket else {
        return Vec::new();
    };
    let _ = socket.set_broadcast(true);
    let _ = socket.set_read_timeout(Some(Duration::from_millis(250)));
    for addr in broadcast_addrs() {
        let _ = socket.send_to(b"%2SRCH\r", SocketAddr::from((addr, 4352)));
    }
    let mut found = Vec::new();
    let deadline = Instant::now() + wait;
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        if let Ok((n, SocketAddr::V4(from))) = socket.recv_from(&mut buf) {
            let text = String::from_utf8_lossy(&buf[..n]).to_ascii_uppercase();
            if text.contains("ACKN") && !found.contains(from.ip()) {
                found.push(*from.ip());
            }
        }
    }
    found
}

/// Identify what (if anything) answers projector control ports at `ip`.
pub fn probe(ip: Ipv4Addr) -> Option<Found> {
    probe_pjlink(ip, 4352).or_else(|| probe_panasonic(ip, 1024))
}

pub fn probe_pjlink(ip: Ipv4Addr, port: u16) -> Option<Found> {
    let mut conn = LineConn::connect_with(&ip.to_string(), port, PROBE_CONNECT, PROBE_IO).ok()?;
    let greeting = conn.read_line().ok()?;
    if !greeting.to_ascii_uppercase().starts_with("PJLINK") {
        return None;
    }
    let needs_password = greeting.split_whitespace().nth(1) == Some("1");
    let mut found = Found {
        ip,
        port,
        protocol: Protocol::PjLink,
        name: String::new(),
        manufacturer: String::new(),
        model: String::new(),
        needs_password,
    };
    if !needs_password {
        let mut ask = |cmd: &str| -> String {
            if conn.send(&format!("%1{cmd} ?\r")).is_err() {
                return String::new();
            }
            match conn.read_line().ok().and_then(|l| parse_reply(&l, cmd)) {
                Some(Reply::Value(v)) => v,
                _ => String::new(),
            }
        };
        found.name = ask("NAME");
        found.manufacturer = ask("INF1");
        found.model = ask("INF2");
    }
    Some(found)
}

pub fn probe_panasonic(ip: Ipv4Addr, port: u16) -> Option<Found> {
    let mut conn = LineConn::connect_with(&ip.to_string(), port, PROBE_CONNECT, PROBE_IO).ok()?;
    let greeting = conn.read_line().ok()?;
    if !greeting.to_ascii_uppercase().starts_with("NTCONTROL") {
        return None;
    }
    Some(Found {
        ip,
        port,
        protocol: Protocol::Panasonic,
        name: String::new(),
        manufacturer: "Panasonic".into(),
        model: String::new(),
        needs_password: greeting.split_whitespace().nth(1) == Some("1"),
    })
}

/// Start a scan in the background. `extra` adds /24 prefixes beyond the local ones
/// (for projectors on another VLAN). `repaint` is called as results arrive.
pub fn start(extra: Vec<[u8; 3]>, repaint: Arc<dyn Fn() + Send + Sync>) -> Scan {
    let mut prefixes = local_prefixes();
    for p in extra {
        if !prefixes.contains(&p) {
            prefixes.push(p);
        }
    }
    let scan = Scan {
        results: Arc::new(Mutex::new(Vec::new())),
        done: Arc::new(AtomicUsize::new(0)),
        total: Arc::new(AtomicUsize::new(prefixes.len() * 254)),
        running: Arc::new(AtomicBool::new(true)),
        subnets: Arc::new(Mutex::new(
            prefixes
                .iter()
                .map(|p| format!("{}.{}.{}.0/24", p[0], p[1], p[2]))
                .collect(),
        )),
    };

    let queue: Vec<Ipv4Addr> = prefixes
        .iter()
        .flat_map(|p| (1..=254u8).map(move |h| Ipv4Addr::new(p[0], p[1], p[2], h)))
        .collect();
    let queue = Arc::new(Mutex::new(queue));

    let add = {
        let scan = scan.clone();
        let repaint = repaint.clone();
        move |f: Found| {
            let mut r = scan.results.lock().unwrap();
            if !r.iter().any(|x| x.ip == f.ip) {
                r.push(f);
                r.sort_by_key(|x| x.ip);
            }
            drop(r);
            repaint();
        }
    };
    let add = Arc::new(add);

    let coordinator = scan.clone();
    std::thread::spawn(move || {
        let mut threads = Vec::new();

        // Broadcast search, then probe whoever answered (may be outside our /24s).
        {
            let add = add.clone();
            threads.push(std::thread::spawn(move || {
                for ip in udp_search(Duration::from_millis(2500)) {
                    if let Some(f) = probe(ip) {
                        add(f);
                    }
                }
            }));
        }

        for _ in 0..WORKERS {
            let queue = queue.clone();
            let add = add.clone();
            let done = coordinator.done.clone();
            let repaint = repaint.clone();
            threads.push(std::thread::spawn(move || loop {
                let Some(ip) = queue.lock().unwrap().pop() else {
                    break;
                };
                if let Some(f) = probe(ip) {
                    add(f);
                }
                if done.fetch_add(1, Ordering::Relaxed).is_multiple_of(16) {
                    repaint();
                }
            }));
        }
        for t in threads {
            let _ = t.join();
        }
        coordinator.running.store(false, Ordering::Relaxed);
        repaint();
    });

    scan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes() {
        assert_eq!(parse_prefix("192.168.10"), Some([192, 168, 10]));
        assert_eq!(parse_prefix(" 10.0.5.0/24 "), Some([10, 0, 5]));
        assert_eq!(parse_prefix("10.0.5.77"), Some([10, 0, 5]));
        assert_eq!(parse_prefix("10.0"), None);
        assert_eq!(parse_prefix("abc"), None);
    }
}
