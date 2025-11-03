use bytes::Bytes;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::service::lease::ProxyLease;
use crate::service::vpn::vpn::VpnCode;
use crate::service::vpn::vpn::VpnConfig;

use super::client::ClientId;
use super::client::ProxyClient;
use super::client::ProxyClientHandle;
use super::client::RemoteInfo;
use super::vpn::router::VPNRouter;

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
    pub async fn new_client(&self, remote: RemoteInfo) -> ProxyClientHandle {
        let mut guard = self.inner.lock().await;
        guard.new_client(remote).await
    }

    pub async fn remove_client(&self, client: ProxyClientHandle) {
        let mut guard = self.inner.lock().await;
        guard.remove_client(client).await
    }

    pub async fn make_vpn(&self, game: &str) -> Arc<VpnConfig> {
        let mut guard = self.inner.lock().await;
        guard.make_vpn(game).await
    }

    pub async fn get_vpn_info(&self, code: &str) -> Option<Arc<VpnConfig>> {
        let mut guard = self.inner.lock().await;
        guard.get_vpn_info(code).await
    }

    pub async fn vpn_route(
        &self,
        code: &VpnCode,
        bind_port: u16,
        relay_tx: UnboundedSender<Bytes>,
    ) -> Option<ProxyLease> {
        let guard = self.inner.lock().await;
        guard.vpn_route(code, bind_port, relay_tx).await
    }

    pub async fn route(
        &self,
        addr: SocketAddr,
        is_udp: bool,
        relay_tx: UnboundedSender<Bytes>,
    ) -> Option<ProxyLease> {
        let guard = self.inner.lock().await;
        guard.route(addr, is_udp, relay_tx).await
    }
    async fn run(self) {
        // Nothing yet
    }
}

pub struct ProxyService {
    next_client_id: ClientId,
    clients: HashMap<ClientId, ProxyClientHandle>,
    vpn_router: VPNRouter,
}

impl ProxyService {
    pub fn new() -> ProxyService {
        Self {
            next_client_id: 1,
            clients: HashMap::new(),
            vpn_router: VPNRouter::new(),
        }
    }

    pub fn start(self) -> (ProxyServiceHandle, JoinHandle<()>) {
        let handle = ProxyServiceHandle::new(self);
        (
            handle.clone(),
            tokio::spawn(async move { handle.run().await }),
        )
    }

    pub async fn new_client(&mut self, remote: RemoteInfo) -> ProxyClientHandle {
        let id = self.next_client_id;
        self.next_client_id += 1;
        let client = ProxyClient::new(id, remote).into_handle();
        self.clients.insert(id, client.clone());
        client
    }

    pub async fn remove_client(&mut self, client: ProxyClientHandle) {
        match self.clients.remove(&client.id()) {
            Some(_) => (),
            None => panic!("Integrity error, removed invalid client"),
        };
    }

    pub async fn make_vpn(&mut self, game: &str) -> Arc<VpnConfig> {
        self.vpn_router.make_vpn(game).await
    }

    pub async fn get_vpn_info(&mut self, hexcode: &str) -> Option<Arc<VpnConfig>> {
        match VpnCode::from(hexcode) {
            Ok(code) => self.vpn_router.get_vpn_info(&code).await,
            Err(_) => {
                // Ignore invalid vpn codes
                None
            }
        }
    }

    pub async fn vpn_route(
        &self,
        code: &VpnCode,
        bind_port: u16,
        relay_tx: UnboundedSender<Bytes>,
    ) -> Option<ProxyLease> {
        self.vpn_router.route(code, bind_port, relay_tx).await
    }

    pub async fn route(
        &self,
        addr: SocketAddr,
        is_udp: bool,
        relay_tx: UnboundedSender<Bytes>,
    ) -> Option<ProxyLease> {
        None
    }
}
