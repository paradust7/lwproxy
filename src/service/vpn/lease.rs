use std::net::SocketAddr;
use std::sync::Arc;

use crate::service::vpn::vpn::Vpn;

use super::packet::VpnPacket;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;

/// A lease is an active route in a VPN.
/// When the lease is dropped, the route is removed.
/// If all routes are dropped, the VPN may also be dropped after inactivity.
pub struct VpnLease {
    pub addr: SocketAddr,
    pub vpn: Arc<Mutex<Vpn>>,
    pub transmit: UnboundedSender<VpnPacket>,
}

impl VpnLease {
    // Send a packet into the VPN
    pub fn send(&self, packet: VpnPacket) -> anyhow::Result<()> {
        self.transmit.send(packet)?;
        Ok(())
    }
}
