use bytes::Buf;
use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::net::SocketAddrV4;

pub struct VpnPacketUdp {
    pub from: SocketAddr,
    pub to: SocketAddr,
    pub data: Bytes,
}
pub enum VpnPacket {
    VpnStop,
    VpnDisconnect(SocketAddr),
    Udp(VpnPacketUdp),
}

impl VpnPacketUdp {
    const MAGIC1: u32 = 0x778B4CF3;

    /// Encode version 1 of the wire protocol
    pub fn encode1(&self) -> Bytes {
        let mut buffer = BytesMut::with_capacity(4 + 4 + 2 + 2 + self.data.len());
        buffer.put_u32(Self::MAGIC1); // Magic prefix
        match self.from.ip() {
            IpAddr::V4(ipv4_addr) => buffer.put_u32(ipv4_addr.to_bits()),
            IpAddr::V6(_) => panic!("IPv6 not handled in encode1"),
        }
        buffer.put_u16(self.from.port());
        buffer.put_u16(self.data.len() as u16);
        buffer.put_slice(&self.data);
        buffer.freeze()
    }

    pub fn decode1(raw: Bytes) -> anyhow::Result<Self> {
        let mut cursor = raw.clone();
        anyhow::ensure!(cursor.len() >= 12, "vpn packet too small");
        anyhow::ensure!(cursor.get_u32() == Self::MAGIC1, "vpn packet invalid magic");
        let [a, b, c, d] = [
            cursor.get_u8(),
            cursor.get_u8(),
            cursor.get_u8(),
            cursor.get_u8(),
        ];
        let dest_ip = Ipv4Addr::new(a, b, c, d);
        let dest_port = cursor.get_u16();
        let data_len = cursor.get_u16() as usize;
        anyhow::ensure!(cursor.len() == data_len, "vpn packet size mismatch");
        Ok(Self {
            from: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0)),
            to: SocketAddr::V4(SocketAddrV4::new(dest_ip, dest_port)),
            data: cursor,
        })
    }
}
