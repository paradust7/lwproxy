use anyhow::Context;
use bytes::Bytes;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::sync::mpsc::UnboundedSender;
use web_transport_quinn::RecvStream;
use web_transport_quinn::Request;
use web_transport_quinn::SendStream;
use web_transport_quinn::Session;

use crate::service::commands::CommandProcessor;
use crate::service::commands::MAX_COMMAND_SIZE;
use crate::service::lease::ProxyLease;
use crate::service::ProxyClientHandle;
use crate::service::ProxyServiceHandle;

pub static READ_BUFFER_SIZE: usize = 16384;

pub struct WebTransportClientRunner {
    service: ProxyServiceHandle,
    lease: Option<ProxyLease>,
    send: Option<SendStream>,
    relay_tx: Option<UnboundedSender<Bytes>>,
    relay_udp: bool,
}

impl WebTransportClientRunner {
    pub fn new(service: ProxyServiceHandle) -> Self {
        Self {
            service,
            lease: None,
            send: None,
            relay_tx: None,
            relay_udp: false,
        }
    }

    pub fn run(self, request: Request) {
        tokio::spawn(async move {
            let result = self.run_inner(request).await;
            if let Err(err) = result {
                log::error!("WebTransport error: {}", err);
            }
        });
    }

    pub async fn run_inner(mut self, request: Request) -> anyhow::Result<()> {
        let session = request.ok().await.context("WebTransport accept")?;
        let client = self
            .service
            .new_client("WebTransport", session.remote_address())
            .await;

        let res = self.handle_session(&client, &session).await;
        self.service.remove_client(client, res.err()).await;
        Ok(())
    }

    async fn handle_session(
        &mut self,
        client: &ProxyClientHandle,
        session: &Session,
    ) -> anyhow::Result<()> {
        let (relay_tx, mut relay_rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
        self.relay_tx = Some(relay_tx);
        let mut recv: Option<BufReader<RecvStream>> = None;
        loop {
            // Wait for a bidirectional stream or datagram.
            tokio::select! {
                res = session.accept_bi() => {
                    // We expect exactly one stream.
                    if self.send.is_some() || recv.is_some() {
                        anyhow::bail!("Not expecting multiple streams");
                    }
                    let (s, r) = res?;
                    self.send = Some(s);
                    recv = Some(BufReader::with_capacity(READ_BUFFER_SIZE, r));
                },
                res = session.accept_uni() => {
                    let _ = res?;
                    anyhow::bail!("Not expecting unidirectional stream");
                },
                data = async { recv.as_mut().unwrap().fill_buf().await }, if recv.is_some() => {
                    let consumed = self.handle_data(client, data?).await?;
                    recv.as_mut().unwrap().consume(consumed);
                },
                res = session.read_datagram() => {
                    let msg = res?;
                    self.handle_datagram(msg).await?
                },
                r = relay_rx.recv() => {
                    self.handle_relay(r, session).await?
                },
            };

            log::info!("echo successful!");
        }
    }

    async fn handle_relay(&mut self, r: Option<Bytes>, session: &Session) -> anyhow::Result<()> {
        match r {
            Some(data) => {
                if data.len() == 0 {
                    // This might signal the end of the lease.
                    if let Some(lease) = &self.lease {
                        if !lease.is_live() {
                            anyhow::bail!("Lease ended");
                        }
                    }
                    return Ok(());
                }
                if self.relay_udp {
                    session.send_datagram(data)?;
                } else {
                    self.send.as_mut().unwrap().write_all(&data).await?;
                }
                return Ok(());
            }
            None => anyhow::bail!("Relay closed"),
        }
    }

    async fn handle_data(
        &mut self,
        client: &ProxyClientHandle,
        data: &[u8],
    ) -> anyhow::Result<usize> {
        if let Some(lease) = &self.lease {
            if self.relay_udp {
                anyhow::bail!("Got data on a datagram-only session");
            }
            lease.send(Bytes::copy_from_slice(data))?;
            return Ok(data.len());
        }
        // When there's no lease, interpret the data as commands separated by \n
        if let Some(nlpos) = data.iter().position(|&b| b == b'\n') {
            let cmd = &data[..nlpos];
            let proc = CommandProcessor::new(&self.service, client);
            let result = proc.handle_command(cmd, &mut self.relay_tx).await?;
            if result.new_lease.is_some() {
                self.lease = result.new_lease;
                self.relay_udp = result.relay_udp;
            }
            self.send
                .as_mut()
                .unwrap()
                .write_all(result.response.as_bytes())
                .await?;
            self.send.as_mut().unwrap().write_all(b"\n").await?;
            return Ok(nlpos);
        }
        if data.len() >= MAX_COMMAND_SIZE {
            anyhow::bail!("Command grew too large in buffer")
        }
        return Ok(0);
    }

    async fn handle_datagram(&mut self, msg: Bytes) -> anyhow::Result<()> {
        if let Some(lease) = &self.lease {
            if !self.relay_udp {
                anyhow::bail!("Got datagram on relay expected to be reliable")
            }
            lease.send(msg)?;
        }
        Ok(())
    }
}
