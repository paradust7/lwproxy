use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::mpsc::UnboundedSender;

use crate::service::lease::ProxyLease;
use crate::service::vpn::vpn::Vpn;
use crate::service::vpn::vpn::VpnCode;
use crate::service::vpn::vpn::VpnConfig;
use crate::service::vpn::vpn::VpnHandle;

pub struct VPNRouter {
    codes: HashMap<VpnCode, VpnHandle>,
}

impl VPNRouter {
    pub fn new() -> Self {
        Self {
            codes: HashMap::new(),
        }
    }
    pub async fn make_vpn(&mut self, game: &str) -> Arc<VpnConfig> {
        let config = Arc::new(VpnConfig {
            server_code: VpnCode::new(),
            client_code: VpnCode::new(),
            game: game.to_owned(),
        });
        let mut vpn = Vpn::new(config.clone());
        vpn.start();
        let vpn = vpn.into_handle();
        self.codes.insert(config.as_ref().server_code, vpn.clone());
        self.codes.insert(config.as_ref().client_code, vpn.clone());
        config
    }

    pub async fn get_vpn_info(&self, code: &VpnCode) -> Option<Arc<VpnConfig>> {
        match self.codes.get(code) {
            Some(vpn) => Some(vpn.config.clone()),
            None => return None,
        }
    }

    pub async fn route(
        &self,
        code: &VpnCode,
        bind_port: u16,
        relay_tx: UnboundedSender<Bytes>,
    ) -> Option<ProxyLease> {
        match self.codes.get(code) {
            Some(vpn) => {
                let ip = vpn.assign_ip(code).await;
                let bind_addr = SocketAddr::new(ip, bind_port);
                let lease = vpn.add_route(bind_addr, relay_tx).await;
                Some(lease)
            }
            None => None,
        }
    }
}
