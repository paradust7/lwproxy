use std::sync::Arc;

use anyhow::Context;
use log;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use tokio::sync::Mutex;
use web_transport_quinn::http::StatusCode;
use web_transport_quinn::RecvStream;
use web_transport_quinn::SendStream;
use web_transport_quinn::Session;

use crate::service::ProxyClientHandle;
use crate::service::ProxyServiceHandle;
use crate::service::RemoteInfo;

use bytes::Bytes;

pub struct WebTransportProxyListenerHandle {
    inner: Arc<Mutex<WebTransportProxyListener>>,
}

pub struct WebTransportProxyListener {
    service: ProxyServiceHandle,
    wtserver: web_transport_quinn::Server,
}

impl WebTransportProxyListener {
    pub fn try_new(
        service: ProxyServiceHandle,
        bind_addr: std::net::SocketAddr,
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> anyhow::Result<Self> {
        let wtserver = web_transport_quinn::ServerBuilder::new()
            .with_addr(bind_addr)
            .with_certificate(chain, key)?;
        log::info!("WebTransport listening on {}", bind_addr);
        Ok(Self { service, wtserver })
    }

    pub async fn run(&mut self) {
        // Accept new connections.
        let mut wt_conn_id = 0;
        while let Some(conn) = self.wtserver.accept().await {
            wt_conn_id += 1;
            let service = self.service.clone();
            tokio::spawn(async move {
                let err = Self::run_conn(service, wt_conn_id, conn).await;
                if let Err(err) = err {
                    log::error!("run_conn uncaught error: {err}")
                }
            });
        }
    }

    async fn run_conn(
        service: ProxyServiceHandle,
        wt_conn_id: u64,
        request: web_transport_quinn::Request,
    ) -> anyhow::Result<()> {
        log::info!("WT{}: New connection. url={}", wt_conn_id, request.url());

        if request.url().path() != "/lwproxy" {
            log::error!("WT{}: Rejecting bad path", wt_conn_id);
            request.close(StatusCode::NOT_FOUND).await?;
            return Ok(());
        }
        // Accept the session.
        let session = request.ok().await.context("failed to accept session")?;
        log::info!(
            "WT{}: Started session for remote={}",
            wt_conn_id,
            session.remote_address()
        );

        // Run the session
        run_session(wt_conn_id, service, &session).await?;
        Ok(())
    }
}

async fn run_session(
    wt_conn_id: u64,
    service: ProxyServiceHandle,
    session: &Session,
) -> anyhow::Result<()> {
    let remote_addr = session.remote_address();
    let client = service
        .new_client(RemoteInfo::WebTransport(remote_addr))
        .await;
    log::info!("WT{}: Adding client C{}", wt_conn_id, client.id());
    loop {
        let res = handle_session_event(client.clone(), &session).await;
        if let Err(err) = res {
            log::info!("WT{}: Removing client C{}", wt_conn_id, client.id());
            service.remove_client(client).await;
            return Err(err);
        }
    }
}

async fn handle_command(client: &ProxyClientHandle, cmd: &Bytes) {}

async fn handle_session_event(client: ProxyClientHandle, session: &Session) -> anyhow::Result<()> {
    let mut command_stream: Option<(SendStream, RecvStream)> = None;
    loop {
        // Wait for a bidirectional stream or datagram.
        tokio::select! {
            res = session.accept_bi() => {
                let (mut send, mut recv) = res?;
                // Ignore streams
                let _ = recv.stop(404);
                let _ = send.finish();
            },
            res = session.accept_uni() => {
                let mut recv = res?;
                // Ignore streams
                let _ = recv.stop(404);
            },
            Some(cmd) = async {
                match &mut command_stream {
                    Some((send, recv)) => Some(recv.read_chunk(4096, true).await),
                    None => None,
                }
            } => {
                match cmd {
                    Ok(Some(x)) => handle_command(&client, &x.bytes).await,
                    Ok(None) => (),
                    Err(err) => panic!("Unhandled read error: {}", err),
                };
            },
            res = session.read_datagram() => {
                let msg = res?;
                log::info!("accepted datagram");
                log::info!("recv: {}", String::from_utf8_lossy(&msg));

                session.send_datagram(msg.clone())?;
                log::info!("send: {}", String::from_utf8_lossy(&msg));
            },
        };

        log::info!("echo successful!");
    }
}
