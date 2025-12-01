use crate::service::commands::CommandProcessor;
use crate::service::lease::ProxyLease;
use crate::service::ProxyClientHandle;
use crate::service::ProxyServiceHandle;
use bytes::Bytes;
use futures::stream::SplitSink;
use futures::SinkExt;
use futures::StreamExt;
use log;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedSender;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::Utf8Bytes;
use tokio_tungstenite::WebSocketStream;

pub struct WebSocketClientRunner {
    service: ProxyServiceHandle,
    tls: Option<TlsAcceptor>,
    stream: Option<TcpStream>,
    relay_tx: Option<UnboundedSender<Bytes>>,
    lease: Option<ProxyLease>,
}

impl WebSocketClientRunner {
    pub fn new(service: ProxyServiceHandle, tls: Option<TlsAcceptor>, stream: TcpStream) -> Self {
        WebSocketClientRunner {
            service,
            tls,
            stream: Some(stream),
            relay_tx: None,
            lease: None,
        }
    }

    pub fn run(self) {
        tokio::spawn(async move {
            let mut this = self;
            let remote_addr = this.stream.as_ref().unwrap().peer_addr().unwrap();
            let client = this.service.new_client("WebSocket", remote_addr).await;
            let result = this.run_inner(&client).await;
            this.service.remove_client(client, result.err()).await;
        });
    }

    async fn run_inner(&mut self, client: &ProxyClientHandle) -> anyhow::Result<()> {
        let mut stream = self.stream.take().unwrap();
        // Do TLS handshake if needed
        if self.tls.is_some() {
            let tls = self.tls.take().unwrap();
            let mut tls_stream = tls.accept(&mut stream).await?;
            self.run_client(&mut tls_stream, client).await?;
        } else {
            self.run_client(&mut stream, client).await?;
        };
        Ok(())
    }

    async fn run_client<S>(
        &mut self,
        stream: &mut S,
        client: &ProxyClientHandle,
    ) -> anyhow::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let ws_stream = tokio_tungstenite::accept_async(stream).await?;
        let (mut writer, mut reader) = ws_stream.split();
        let (relay_tx, mut relay_rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();
        self.relay_tx = Some(relay_tx);

        loop {
            tokio::select! {
                msg = reader.next() => {
                    match msg {
                        Some(Ok(msg)) => {
                            let did_close = self.handle_message(msg, &mut writer, client).await?;
                            if did_close {
                                return Ok(());
                            }
                        },
                        Some(Err(err)) => {
                            // Websocket errors are always fatal.
                            anyhow::bail!(err);
                        },
                        None => {
                            anyhow::bail!("Already closed?");
                        },
                    }
                },
                r = relay_rx.recv() => {
                    match r {
                        Some(data) => {
                            if data.len() == 0 {
                                // This might signal the end of the lease.
                                if let Some(lease) = &self.lease {
                                    if !lease.is_live() {
                                        let response = CloseFrame {
                                            code: CloseCode::Normal,
                                            reason: Utf8Bytes::from_static("Done"),
                                        };
                                        writer.send(Message::Close(Some(response))).await?;
                                        writer.close().await?;
                                        return Ok(());
                                    }
                                }
                            }
                            writer.send(Message::Binary(data)).await?;
                        }
                        None => {
                            // The relay is shutdown to indicate close
                            let response = CloseFrame {
                                code: CloseCode::Normal,
                                reason: Utf8Bytes::from_static("Done"),
                            };
                            writer.send(Message::Close(Some(response))).await?;
                            writer.close().await?;
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    async fn handle_message<S>(
        &mut self,
        msg: Message,
        writer: &mut SplitSink<WebSocketStream<S>, Message>,
        client: &ProxyClientHandle,
    ) -> anyhow::Result<bool>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        match msg {
            Message::Text(utf8_bytes) => {
                if self.lease.is_some() {
                    anyhow::bail!("Command sent at improper time")
                }
                let proc = CommandProcessor::new(&self.service, client);
                let result = proc
                    .handle_command(utf8_bytes.as_bytes(), &mut self.relay_tx)
                    .await?;
                writer.send(Message::Text(result.response.into())).await?;
                self.lease = result.new_lease;
            }
            Message::Binary(bytes) => self.handle_datagram(bytes).await?,
            Message::Ping(bytes) => writer.send(Message::Pong(bytes)).await?,
            Message::Pong(_bytes) => {}
            Message::Close(close_frame) => match close_frame {
                Some(frame) => {
                    writer.send(Message::Close(Some(frame.clone()))).await?;
                    log::info!(
                        "C{}: Received close frame (code={}, reason={})",
                        client.id(),
                        frame.code,
                        frame.reason
                    );
                    return Ok(true);
                }
                None => {
                    let frame = CloseFrame {
                        code: CloseCode::Normal,
                        reason: Utf8Bytes::from_static("Done"),
                    };
                    writer.send(Message::Close(Some(frame))).await?;
                    log::info!("C{}: Received blank close frame", client.id());
                    return Ok(true);
                }
            },
            Message::Frame(_) => panic!("Unexpected frame"),
        }
        Ok(false)
    }

    async fn handle_datagram(&mut self, bytes: Bytes) -> anyhow::Result<()> {
        match &self.lease {
            Some(lease) => {
                lease.send(bytes)?;
                Ok(())
            }
            None => {
                anyhow::bail!("Datagram without target");
            }
        }
    }
}
