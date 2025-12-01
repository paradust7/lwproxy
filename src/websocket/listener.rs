use super::runner::WebSocketProxyRunner;
use super::runner::WebSocketProxyRunnerHandle;
use crate::service::ProxyServiceHandle;
use crate::settings::TLSCert;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

pub struct WebSocketProxyListener {
    service: ProxyServiceHandle,
    bind_addr: std::net::SocketAddr,
    tls: Option<TlsAcceptor>,
    runner: Option<WebSocketProxyRunnerHandle>,
}

impl WebSocketProxyListener {
    pub async fn new(
        service: ProxyServiceHandle,
        bind_addr: std::net::SocketAddr,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            service,
            bind_addr,
            tls: None,
            runner: None,
        })
    }

    pub async fn new_secure(
        service: ProxyServiceHandle,
        bind_addr: std::net::SocketAddr,
        cert: TLSCert,
    ) -> anyhow::Result<Self> {
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(cert.chain, cert.key)?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        Ok(Self {
            service,
            bind_addr,
            tls: Some(acceptor),
            runner: None,
        })
    }

    pub async fn start(&mut self) -> anyhow::Result<JoinHandle<()>> {
        assert!(self.runner.is_none());
        let listener = TcpListener::bind(&self.bind_addr).await?;
        log::info!("WebSocket server started (bind addr {})", self.bind_addr);
        let runner = WebSocketProxyRunner::new(self.service.clone(), listener, self.tls.clone())
            .run()
            .await;
        self.runner = Some(runner);
        Ok(self.runner.as_mut().unwrap().join.take().unwrap())
    }

    pub async fn stop(&self) -> anyhow::Result<()> {
        if let Some(runner) = &self.runner {
            runner.stop.send(true)?;
        }
        Ok(())
    }
}
