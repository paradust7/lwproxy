use std::collections::HashMap;
use std::net::IpAddr;
use std::net::Ipv6Addr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedSender;

use crate::service::lease::LeaseHandler;
use crate::service::lease::ProxyLease;

use super::code::is_network_address;
use super::code::random_address;
use super::code::Passcode;
use super::packet::Frame;

/// How long an assignment outlives the last port bound on it. A client that
/// is playing holds a port the whole time, so it keeps its address for as
/// long as it stays connected.
static ASSIGNMENT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Largest datagram relayed to or from the open internet.
static MAX_DATAGRAM: usize = 4096;

type RouteKey = (Ipv6Addr, u16);

/// One address handed to one client, and the secret that lets it be used.
struct Assignment {
    passcode: Passcode,
    /// Ports currently bound on this address.
    binds: usize,
    /// When a port was last bound or unbound.
    touched: Instant,
}

struct NetworkState {
    by_address: HashMap<Ipv6Addr, Assignment>,
    by_passcode: HashMap<Passcode, Ipv6Addr>,
    routes: HashMap<RouteKey, UnboundedSender<Bytes>>,
}

impl NetworkState {
    /// Forgets assignments that have had nothing bound for a day. Called
    /// whenever an address is handed out, so the table is bounded by how
    /// busy the proxy is rather than by how long it has been up.
    fn expire(&mut self) {
        let now = Instant::now();
        let stale: Vec<(Ipv6Addr, Passcode)> = self
            .by_address
            .iter()
            .filter(|(_, a)| a.binds == 0 && now.duration_since(a.touched) >= ASSIGNMENT_TTL)
            .map(|(address, a)| (*address, a.passcode.clone()))
            .collect();
        for (address, passcode) in stale {
            self.by_address.remove(&address);
            self.by_passcode.remove(&passcode);
            log::info!("Assignment {} expired", address);
        }
    }
}

/// The private network shared by every client of this proxy.
///
/// A client is handed an address and a passcode, and binds ports on that
/// address. A datagram addressed to a bound port goes to whoever bound it;
/// anything else goes out to the internet through a translated socket, and
/// replies come back the same way.
pub struct Network {
    state: Mutex<NetworkState>,
}

impl Network {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(NetworkState {
                by_address: HashMap::new(),
                by_passcode: HashMap::new(),
                routes: HashMap::new(),
            }),
        }
    }

    /// Hands out a fresh address and the passcode that goes with it. Only
    /// the proxy chooses addresses, so a client cannot claim one it was
    /// not given.
    pub fn assign(&self) -> (Ipv6Addr, Passcode) {
        let mut guard = self.state.lock().unwrap();
        guard.expire();
        loop {
            let address = random_address();
            let passcode = Passcode::new();
            if guard.by_address.contains_key(&address) || guard.by_passcode.contains_key(&passcode)
            {
                continue;
            }
            guard.by_address.insert(
                address,
                Assignment {
                    passcode: passcode.clone(),
                    binds: 0,
                    touched: Instant::now(),
                },
            );
            guard.by_passcode.insert(passcode.clone(), address);
            log::info!("Assigned {}", address);
            return (address, passcode);
        }
    }

    /// Claims a port on the address the passcode was issued for.
    ///
    /// Fails if the assignment is gone, which is how a client finds out that
    /// it expired or that the proxy restarted, and its cue to ask for a new
    /// address.
    pub fn bind(
        self: &Arc<Self>,
        passcode: &Passcode,
        port: u16,
        relay: UnboundedSender<Bytes>,
    ) -> anyhow::Result<ProxyLease> {
        anyhow::ensure!(port != 0, "Cannot bind port 0");
        let key;
        {
            let mut guard = self.state.lock().unwrap();
            let state = &mut *guard;
            let address = *state
                .by_passcode
                .get(passcode)
                .context("Unknown or expired passcode")?;
            key = (address, port);
            anyhow::ensure!(
                !state.routes.contains_key(&key),
                "Port {} is already bound on {}",
                port,
                address
            );
            let assignment = state
                .by_address
                .get_mut(&address)
                .context("Assignment is missing")?;
            assignment.binds += 1;
            assignment.touched = Instant::now();
            state.routes.insert(key, relay.clone());
        }
        log::info!("Bound {} port {}", key.0, key.1);
        let (egress_tx, egress_rx) = mpsc::unbounded_channel();
        tokio::spawn(translate(key, egress_rx, relay));
        Ok(ProxyLease::new(BindHandler {
            network: self.clone(),
            key,
            egress: egress_tx,
        }))
    }

    fn unbind(&self, key: &RouteKey) {
        let mut guard = self.state.lock().unwrap();
        let state = &mut *guard;
        if state.routes.remove(key).is_none() {
            return;
        }
        if let Some(assignment) = state.by_address.get_mut(&key.0) {
            assignment.binds = assignment.binds.saturating_sub(1);
            assignment.touched = Instant::now();
        }
    }

    /// Hands one datagram to whoever bound the address it is addressed to.
    /// A datagram with nowhere to go is dropped, as any other UDP datagram
    /// sent to nobody would be.
    fn deliver(&self, from: RouteKey, to: RouteKey, data: Bytes) -> anyhow::Result<()> {
        let out = Frame::new(from.0, from.1, data).encode()?;
        let guard = self.state.lock().unwrap();
        if let Some(relay) = guard.routes.get(&to) {
            // A relay that has gone away belongs to a client on its way out.
            // Its own lease removes the route.
            let _ = relay.send(out);
        }
        Ok(())
    }
}

/// Carries one bound port's traffic to and from the open internet.
///
/// One socket per family serves every peer, so a client keeps the same
/// source port whoever it talks to. The task ends when the lease that owns
/// it is dropped and the channel closes.
async fn translate(
    bound: RouteKey,
    mut outgoing: mpsc::UnboundedReceiver<(SocketAddr, Bytes)>,
    relay: UnboundedSender<Bytes>,
) {
    let mut v4: Option<UdpSocket> = None;
    let mut v6: Option<UdpSocket> = None;
    let mut buf4 = vec![0u8; MAX_DATAGRAM];
    let mut buf6 = vec![0u8; MAX_DATAGRAM];
    loop {
        tokio::select! {
            out = outgoing.recv() => {
                let (peer, data) = match out {
                    Some(out) => out,
                    // The lease was dropped.
                    None => return,
                };
                let socket = if peer.is_ipv4() { &mut v4 } else { &mut v6 };
                if socket.is_none() {
                    let local = if peer.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" };
                    match UdpSocket::bind(local).await {
                        Ok(opened) => *socket = Some(opened),
                        Err(err) => {
                            log::info!("{}:{} cannot reach {}: {}", bound.0, bound.1, peer, err);
                            continue;
                        }
                    }
                }
                if let Err(err) = socket.as_ref().unwrap().send_to(&data, peer).await {
                    log::info!("{}:{} send to {} failed: {}", bound.0, bound.1, peer, err);
                }
            },
            r = async { v4.as_ref().unwrap().recv_from(&mut buf4).await }, if v4.is_some() => {
                if !relay_reply(r, &buf4, &relay) {
                    return;
                }
            },
            r = async { v6.as_ref().unwrap().recv_from(&mut buf6).await }, if v6.is_some() => {
                if !relay_reply(r, &buf6, &relay) {
                    return;
                }
            },
        }
    }
}

/// Wraps one datagram from the internet as a frame for the client. Returns
/// false when the client is gone.
fn relay_reply(
    received: std::io::Result<(usize, SocketAddr)>,
    buf: &[u8],
    relay: &UnboundedSender<Bytes>,
) -> bool {
    let (len, peer) = match received {
        Ok(received) => received,
        Err(err) => {
            log::info!("Translated socket read failed: {}", err);
            return true;
        }
    };
    let frame = match Frame::to_peer(peer, Bytes::copy_from_slice(&buf[..len])).encode() {
        Ok(frame) => frame,
        Err(err) => {
            log::info!("Dropping datagram from {}: {}", peer, err);
            return true;
        }
    };
    relay.send(frame).is_ok()
}

/// An active claim on one port. Dropping it releases the port and stops the
/// translation task.
struct BindHandler {
    network: Arc<Network>,
    key: RouteKey,
    egress: mpsc::UnboundedSender<(SocketAddr, Bytes)>,
}

impl LeaseHandler for BindHandler {
    fn send(&self, data: Bytes) -> anyhow::Result<()> {
        let frame = Frame::decode(data).context("Bad frame from client")?;
        let peer = frame.peer();
        if let IpAddr::V6(v6) = peer.ip() {
            if is_network_address(&v6) {
                return self
                    .network
                    .deliver(self.key, (v6, peer.port()), frame.data);
            }
        }
        self.egress.send((peer, frame.data))?;
        Ok(())
    }

    fn close(&self) -> anyhow::Result<()> {
        self.network.unbind(&self.key);
        Ok(())
    }

    fn is_live(&self) -> bool {
        // A bound port stays live until its client goes away, which drops
        // the lease.
        true
    }
}
