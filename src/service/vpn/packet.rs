use bytes::Bytes;
use std::net::SocketAddr;

pub enum VpnProtocol {
    Udp,
    Tcp,
}
pub struct VpnPacket {
    pub proto: VpnProtocol,
    pub from: SocketAddr,
    pub to: SocketAddr,
    pub data: Bytes,
}
