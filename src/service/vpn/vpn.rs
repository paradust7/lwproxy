use std::collections::HashMap;
use std::fmt;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use chrono::DateTime;
use chrono::Utc;
use rand::Rng;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::service::lease::ProxyLease;

use super::lease::VpnLeaseHandler;
use super::packet::VpnPacket;

#[derive(Hash, PartialEq, Eq, Copy, Clone)]
pub struct VpnCode {
    code: [u8; 6],
}

impl VpnCode {
    pub fn new() -> Self {
        let mut rng = rand::rngs::ThreadRng::default();
        let mut code = [0u8; 6];
        rng.fill(&mut code);
        Self { code }
    }
    //        let code = code.iter().map(|b| format!("{:02x}", b)).collect();
    pub fn from(hexcode: &str) -> anyhow::Result<Self> {
        let mut code = [0u8; 6];
        hex::decode_to_slice(hexcode, &mut code)?;
        Ok(Self { code })
    }
}

impl fmt::Display for VpnCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", hex::encode(self.code))
    }
}

pub struct VpnConfig {
    pub server_code: VpnCode,
    pub client_code: VpnCode,
    pub game: String,
}

#[derive(Clone)]
pub struct VpnHandle {
    pub config: Arc<VpnConfig>,
    inner: Arc<Mutex<Vpn>>,
    runtime: Arc<Mutex<VpnRuntime>>,
}

impl VpnHandle {
    /// Add a route so that udp packets sent to `addr` go to the relay channel.
    pub async fn add_route(
        &self,
        addr: SocketAddr,
        relay: mpsc::UnboundedSender<Bytes>,
    ) -> ProxyLease {
        let target = Arc::new(VpnTarget { addr, relay });
        let mut guard = self.runtime.lock().await;
        guard.rtable.insert(addr, target);
        drop(guard);

        let transmitter = {
            let guard = self.inner.lock().await;
            guard.transmitter.as_ref().map(|t| t.clone()).unwrap()
        };

        ProxyLease::new(VpnLeaseHandler {
            addr,
            vpn: self.inner.clone(),
            transmit: transmitter,
        })
    }

    pub async fn assign_ip(&self, code: &VpnCode) -> IpAddr {
        let mut guard = self.inner.lock().await;
        guard.assign_ip(code).await
    }
}

struct VpnTarget {
    addr: SocketAddr,
    relay: mpsc::UnboundedSender<Bytes>,
}

pub struct Vpn {
    pub config: Arc<VpnConfig>,
    pub created: DateTime<Utc>,
    transmitter: Option<mpsc::UnboundedSender<VpnPacket>>,
    // This is only used by the vpn task.
    receiver: Option<mpsc::UnboundedReceiver<VpnPacket>>,
    runtime: Arc<Mutex<VpnRuntime>>,
    join: Option<JoinHandle<()>>,
}

pub struct VpnRuntime {
    rtable: HashMap<SocketAddr, Arc<VpnTarget>>,
    last_packet: DateTime<Utc>,
}

impl Vpn {
    pub fn new(config: Arc<VpnConfig>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            config,
            created: Utc::now(),
            transmitter: Some(tx),
            receiver: Some(rx),
            runtime: Arc::new(Mutex::new(VpnRuntime {
                rtable: HashMap::new(),
                last_packet: Utc::now(),
            })),
            join: None,
        }
    }

    pub fn into_handle(self) -> VpnHandle {
        let runtime = self.runtime.clone();
        VpnHandle {
            config: self.config.clone(),
            inner: Arc::new(Mutex::new(self)),
            runtime,
        }
    }

    pub fn start(&mut self) {
        let config = self.config.clone();
        let runtime = self.runtime.clone();
        let mut receiver = self.receiver.take().unwrap();
        log::info!(
            "VPN {}:{} started",
            config.server_code,
            self.config.client_code
        );
        self.join = Some(tokio::spawn(async move {
            while let Some(packet) = receiver.recv().await {
                match &packet {
                    VpnPacket::VpnStop => break,
                    VpnPacket::VpnDisconnect(addr) => {
                        let mut guard = runtime.lock().await;
                        guard.rtable.remove(&addr);
                    }
                    VpnPacket::Udp(vpn_packet_udp) => {
                        let mut guard = runtime.lock().await;
                        let target_addr = vpn_packet_udp.to;
                        let mut failed = false;
                        if let Some(target) = guard.rtable.get(&target_addr) {
                            if target.relay.send(vpn_packet_udp.encode1()).is_err() {
                                failed = true;
                            }
                        }
                        if failed {
                            // Client disconnected
                            guard.rtable.remove(&target_addr);
                        }
                    }
                }
            }
            log::info!("VPN {}:{} stopped", config.server_code, config.client_code);
        }));
    }

    pub async fn stop(&mut self) -> anyhow::Result<()> {
        if let Some(tx) = &self.transmitter {
            let _ = tx.send(VpnPacket::VpnStop);
        };
        self.join.take().unwrap().await?;
        Ok(())
    }

    pub async fn assign_ip(&mut self, code: &VpnCode) -> IpAddr {
        if code == &self.config.server_code {
            "172.16.0.1".parse().unwrap()
        } else {
            assert!(code == &self.config.client_code);
            // There's a microscopic chance of collision here.
            // TODO: It would be better if IPs were assigned sequentially.
            let mut rng = rand::rngs::ThreadRng::default();
            let b: u8 = rng.random_range(16..=32);
            let c: u8 = rng.random_range(1..=254);
            let d: u8 = rng.random_range(1..=254);
            IpAddr::V4(Ipv4Addr::new(172, b, c, d))
        }
    }
}
