use std::net::IpAddr;

use super::packet::VpnPacket;
use tokio::sync::mpsc;

pub type VpnLeaseId = u64;

pub struct VpnClientLease {
    id: VpnLeaseId,
    ip: IpAddr,
    transmit: mpsc::Sender<(VpnLeaseId, IpAddr, VpnPacket)>,
}

impl VpnClientLease {
    // Send a packet from the client into the VPN
    // If this returns false, the lease is expired.
    pub async fn transmit(&self, packet: VpnPacket) -> bool {
        match self.transmit.send((self.id, self.ip, packet)).await {
            Ok(_) => true,
            Err(_) => false,
        }
    }
}

pub struct VpnClient {
    id: VpnLeaseId,
    ip: IpAddr,
    relay: mpsc::Sender<VpnPacket>,
}
