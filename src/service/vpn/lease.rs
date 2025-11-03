use std::net::SocketAddr;
use std::sync::Arc;

use crate::service::lease::LeaseHandler;
use crate::service::vpn::packet::VpnPacketUdp;
use crate::service::vpn::vpn::Vpn;

use super::packet::VpnPacket;
use anyhow::Context;
use bytes::Bytes;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;

/// A lease is an active route in a VPN.
/// When the lease is dropped, the route is removed.
/// If all routes are dropped, the VPN may also be dropped after inactivity.
pub struct VpnLeaseHandler {
    pub addr: SocketAddr,
    pub vpn: Arc<Mutex<Vpn>>,
    pub transmit: UnboundedSender<VpnPacket>,
}

impl LeaseHandler for VpnLeaseHandler {
    // Send a packet into the VPN from the client
    fn send(&self, data: Bytes) -> anyhow::Result<()> {
        let mut packet = VpnPacketUdp::decode1(data).context("VpnPacketUdp parse error")?;
        packet.from = self.addr;
        self.transmit.send(VpnPacket::Udp(packet))?;
        Ok(())
    }

    fn close(&self) -> anyhow::Result<()> {
        self.transmit.send(VpnPacket::VpnDisconnect(self.addr))?;
        Ok(())
    }

    fn is_live(&self) -> bool {
        !self.transmit.is_closed()
    }
}
