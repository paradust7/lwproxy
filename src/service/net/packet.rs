use std::net::IpAddr;
use std::net::Ipv6Addr;
use std::net::SocketAddr;

use bytes::Buf;
use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;

/// One datagram on a bound link, carrying the address of the other end.
///
/// Sent by a client, the address is where the datagram is going. Sent to a
/// client, it is where the datagram came from. The header has a fixed shape:
///
///   magic (4) | address (16) | port (2) | payload length (2) | payload
///
/// The address field is always 16 bytes. A peer out on the IPv4 internet is
/// carried as an IPv4-mapped address, ::ffff:1.2.3.4, and both ends unmap it
/// before handing it to anything else.
pub struct Frame {
    pub peer_ip: Ipv6Addr,
    pub peer_port: u16,
    pub data: Bytes,
}

impl Frame {
    const MAGIC: u32 = 0x778B4CF6;
    pub const HEADER_LEN: usize = 4 + 16 + 2 + 2;

    pub fn new(peer_ip: Ipv6Addr, peer_port: u16, data: Bytes) -> Self {
        Self {
            peer_ip,
            peer_port,
            data,
        }
    }

    /// A frame for a datagram to or from `peer`, mapping IPv4 into the
    /// address field.
    pub fn to_peer(peer: SocketAddr, data: Bytes) -> Self {
        let peer_ip = match peer.ip() {
            IpAddr::V4(v4) => v4.to_ipv6_mapped(),
            IpAddr::V6(v6) => v6,
        };
        Self::new(peer_ip, peer.port(), data)
    }

    /// Where this frame is going, or where it came from, unmapping an
    /// IPv4-mapped address back to the IPv4 address it stands for.
    pub fn peer(&self) -> SocketAddr {
        match self.peer_ip.to_ipv4_mapped() {
            Some(v4) => SocketAddr::new(IpAddr::V4(v4), self.peer_port),
            None => SocketAddr::new(IpAddr::V6(self.peer_ip), self.peer_port),
        }
    }

    pub fn encode(&self) -> anyhow::Result<Bytes> {
        let len: u16 = self
            .data
            .len()
            .try_into()
            .map_err(|_| anyhow::anyhow!("datagram too large to encapsulate"))?;
        let mut buffer = BytesMut::with_capacity(Self::HEADER_LEN + self.data.len());
        buffer.put_u32(Self::MAGIC);
        buffer.put_slice(&self.peer_ip.octets());
        buffer.put_u16(self.peer_port);
        buffer.put_u16(len);
        buffer.put_slice(&self.data);
        Ok(buffer.freeze())
    }

    pub fn decode(raw: Bytes) -> anyhow::Result<Self> {
        let mut cursor = raw;
        anyhow::ensure!(cursor.len() >= Self::HEADER_LEN, "frame too small");
        anyhow::ensure!(cursor.get_u32() == Self::MAGIC, "frame has invalid magic");
        let mut octets = [0u8; 16];
        cursor.copy_to_slice(&mut octets);
        let peer_ip = Ipv6Addr::from(octets);
        let peer_port = cursor.get_u16();
        let len = cursor.get_u16() as usize;
        anyhow::ensure!(cursor.len() == len, "frame size mismatch");
        Ok(Self {
            peer_ip,
            peer_port,
            data: cursor,
        })
    }
}
