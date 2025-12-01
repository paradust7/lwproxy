use tokio::task::JoinHandle;

use crate::service::ProxyServiceHandle;
use crate::webtransport::client::WebTransportClientRunner;

pub struct WebTransportProxyRunnerHandle {
    pub(super) stop: tokio::sync::watch::Sender<bool>,
    pub(super) join: Option<JoinHandle<()>>,
}

pub struct WebTransportProxyRunner {
    service: ProxyServiceHandle,
    wtserver: web_transport_quinn::Server,
}

impl WebTransportProxyRunner {
    pub fn new(service: ProxyServiceHandle, wtserver: web_transport_quinn::Server) -> Self {
        Self { service, wtserver }
    }

    pub async fn run(self) -> WebTransportProxyRunnerHandle {
        let (stop_send, stop_recv) = tokio::sync::watch::channel(false);
        WebTransportProxyRunnerHandle {
            stop: stop_send,
            join: Some(tokio::spawn(async move {
                Self::run_inner(self, stop_recv).await;
            })),
        }
    }

    async fn run_inner(mut self, stop_recv: tokio::sync::watch::Receiver<bool>) {
        // Accept new connections.
        while let Some(request) = self.wtserver.accept().await {
            let service = self.service.clone();
            WebTransportClientRunner::new(service).run(request);
        }
    }
}
