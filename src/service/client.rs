use bytes::Bytes;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RemoteInfo {
    WebSocket(std::net::SocketAddr),
    WebTransport(std::net::SocketAddr),
}
pub type ClientId = u64;

#[derive(Clone)]
pub struct ProxyClientHandle {
    id: ClientId,
    inner: Arc<Mutex<ProxyClient>>,
}

impl ProxyClientHandle {
    pub fn id(&self) -> u64 {
        self.id
    }
}

pub struct ProxyClient {
    id: ClientId,
    remote: RemoteInfo,
}

impl ProxyClient {
    pub fn new(id: ClientId, remote: RemoteInfo) -> Self {
        Self { id, remote }
    }

    pub fn into_handle(self) -> ProxyClientHandle {
        ProxyClientHandle {
            id: self.id,
            inner: Arc::new(Mutex::new(self)),
        }
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn clone(&self) -> Self {
        ProxyClient {
            id: self.id,
            remote: self.remote,
        }
    }

    pub fn handle_datagram(&self, buf: Bytes) {
        panic!("unimpl")
    }
}
