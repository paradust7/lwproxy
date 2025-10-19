use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use super::client::ClientId;
use super::client::ProxyClient;
use super::client::ProxyClientHandle;
use super::client::RemoteInfo;

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

    pub async fn make_vpn(&self, game: &str) -> (String, String) {
        let mut guard = self.inner.lock().await;
        guard.make_vpn(game).await
    }

    async fn run(self) {
        // Nothing yet
    }
}

pub struct ProxyService {
    next_client_id: ClientId,
    clients: HashMap<ClientId, ProxyClientHandle>,
}

impl ProxyService {
    pub fn new() -> ProxyService {
        Self {
            next_client_id: 1,
            clients: HashMap::new(),
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

    pub async fn make_vpn(&mut self, game: &str) -> (String, String) {
        todo!()
    }
}
