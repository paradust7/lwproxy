use bytes::Bytes;
use std::collections::HashMap;
use std::net::IpAddr;
use std::net::Ipv6Addr;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::service::connectproxy::ConnectProxy;
use crate::service::dnsproxy::DNSProxy;
use crate::service::lease::ProxyLease;
use crate::service::net::code::Passcode;
use crate::service::net::code::PROXY_ADDRESS;
use crate::service::net::network::Network;

use super::client::ClientId;
use super::client::ProxyClient;
use super::client::ProxyClientHandle;

// The proxy's own services, reached over TCP at its address on the private
// network.
static CONNECT_PROXY: SocketAddr = SocketAddr::new(IpAddr::V6(PROXY_ADDRESS), 8080);
static DNS_PROXY: SocketAddr = SocketAddr::new(IpAddr::V6(PROXY_ADDRESS), 53);

#[derive(Clone)]
pub struct ProxyServiceHandle {
    inner: Arc<Mutex<ProxyService>>,
}

impl ProxyServiceHandle {
    pub fn new(inner: ProxyService) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
        }
    }
    pub async fn new_client(&self, protocol: &str, remote_addr: SocketAddr) -> ProxyClientHandle {
        let mut guard = self.inner.lock().await;
        guard.new_client(protocol, remote_addr).await
    }

    pub async fn remove_client(&self, client: ProxyClientHandle, err: Option<anyhow::Error>) {
        let mut guard = self.inner.lock().await;
        guard.remove_client(client, err).await
    }

    pub async fn assign(&self) -> (Ipv6Addr, Passcode) {
        let guard = self.inner.lock().await;
        guard.network.assign()
    }

    pub async fn bind(
        &self,
        passcode: &Passcode,
        bind_port: u16,
        relay_tx: UnboundedSender<Bytes>,
    ) -> anyhow::Result<ProxyLease> {
        let guard = self.inner.lock().await;
        guard.network.bind(passcode, bind_port, relay_tx)
    }

    pub async fn route(
        &self,
        addr: SocketAddr,
        relay_tx: UnboundedSender<Bytes>,
    ) -> anyhow::Result<ProxyLease> {
        let guard = self.inner.lock().await;
        guard.route(addr, relay_tx).await
    }
    async fn run(self) {
        // Nothing yet
    }
}

pub struct ProxyService {
    clients: HashMap<ClientId, ProxyClientHandle>,
    network: Arc<Network>,
}

impl ProxyService {
    pub fn new() -> ProxyService {
        Self {
            clients: HashMap::new(),
            network: Arc::new(Network::new()),
        }
    }

    pub fn start(self) -> (ProxyServiceHandle, JoinHandle<()>) {
        let handle = ProxyServiceHandle::new(self);
        (
            handle.clone(),
            tokio::spawn(async move { handle.run().await }),
        )
    }

    pub async fn new_client(
        &mut self,
        protocol: &str,
        remote_addr: SocketAddr,
    ) -> ProxyClientHandle {
        let client = ProxyClient::new(remote_addr).into_handle();
        self.clients.insert(client.id(), client.clone());
        log::info!(
            "C{}: Client connected from {} using {}",
            client.id(),
            remote_addr,
            protocol
        );
        client
    }

    pub async fn remove_client(&mut self, client: ProxyClientHandle, err: Option<anyhow::Error>) {
        let id = client.id();
        match self.clients.remove(&id) {
            Some(_) => {
                if let Some(err) = err {
                    log::info!("C{}: Disconnected due to error ({})", id, err);
                } else {
                    log::info!("C{}: Disconnected", id);
                }
            }
            None => panic!("Integrity error, removed invalid client"),
        };
    }

    /// PROXY reaches the proxy's own services and nothing else. Everything
    /// else a client talks to is UDP, which goes over a bound link.
    pub async fn route(
        &self,
        addr: SocketAddr,
        relay_tx: UnboundedSender<Bytes>,
    ) -> anyhow::Result<ProxyLease> {
        if addr == CONNECT_PROXY {
            Ok(ConnectProxy::new(relay_tx).into_lease())
        } else if addr == DNS_PROXY {
            Ok(DNSProxy::new(relay_tx).into_lease())
        } else {
            anyhow::bail!("Only the proxy's own services can be reached with PROXY")
        }
    }
}
