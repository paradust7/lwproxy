use log;
use tokio::task::JoinHandle;

use crate::service::ProxyServiceHandle;
use crate::settings::TLSCert;
use crate::webtransport::runner::WebTransportProxyRunner;
use crate::webtransport::runner::WebTransportProxyRunnerHandle;

pub struct WebTransportProxyListener {
    bind_addr: std::net::SocketAddr,
    service: ProxyServiceHandle,
    cert: TLSCert,
    runner: Option<WebTransportProxyRunnerHandle>,
}

impl WebTransportProxyListener {
    pub async fn new(
        service: ProxyServiceHandle,
        bind_addr: std::net::SocketAddr,
        cert: TLSCert,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            bind_addr,
            service,
            cert,
            runner: None,
        })
    }

    pub async fn start(&mut self) -> anyhow::Result<JoinHandle<()>> {
        assert!(self.runner.is_none());
        let wtserver = web_transport_quinn::ServerBuilder::new()
            .with_addr(self.bind_addr)
            .with_certificate(self.cert.chain.clone(), self.cert.key.clone_key())?;
        log::info!("WebTransport listening on {}", self.bind_addr);

        let runner = WebTransportProxyRunner::new(self.service.clone(), wtserver)
            .run()
            .await;
        self.runner = Some(runner);
        Ok(self.runner.as_mut().unwrap().join.take().unwrap())
    }
}
