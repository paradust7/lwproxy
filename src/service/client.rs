use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::Mutex;

use std::sync::atomic::AtomicU64;
static CLIENT_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    remote_addr: SocketAddr,
}

impl ProxyClient {
    pub fn new(remote_addr: SocketAddr) -> Self {
        let id = CLIENT_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self { id, remote_addr }
    }

    pub fn into_handle(self) -> ProxyClientHandle {
        ProxyClientHandle {
            id: self.id,
            inner: Arc::new(Mutex::new(self)),
        }
    }

    pub fn id(&self) -> ClientId {
        self.id
    }
}
