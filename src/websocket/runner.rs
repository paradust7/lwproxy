use crate::service::ProxyServiceHandle;
use crate::websocket::client::WebSocketClientRunner;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

pub struct WebSocketProxyRunnerHandle {
    pub(super) stop: tokio::sync::watch::Sender<bool>,
    pub(super) join: JoinHandle<()>,
}

pub struct WebSocketProxyRunner {
    service: ProxyServiceHandle,
    listener: TcpListener,
    tls: Option<TlsAcceptor>,
}

impl WebSocketProxyRunner {
    pub fn new(
        service: ProxyServiceHandle,
        listener: TcpListener,
        tls: Option<TlsAcceptor>,
    ) -> Self {
        WebSocketProxyRunner {
            service,
            listener,
            tls,
        }
    }

    pub async fn run(self) -> WebSocketProxyRunnerHandle {
        let (stop_send, stop_recv) = tokio::sync::watch::channel(false);
        WebSocketProxyRunnerHandle {
            stop: stop_send,
            join: tokio::spawn(async move {
                Self::run_inner(self, stop_recv);
            }),
        }
    }

    async fn run_inner(self, stop_recv: tokio::sync::watch::Receiver<bool>) {
        //tokio::select! {}
        while let Ok((stream, _)) = self.listener.accept().await {
            let service = self.service.clone();
            let tls = self.tls.clone();
            WebSocketClientRunner::new(service, tls, stream).run();
        }
    }
}
