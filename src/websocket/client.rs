use bytes::Bytes;
use futures::stream::SplitSink;
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::WebSocketStream;

use crate::service::ProxyServiceHandle;
use futures::SinkExt;
use futures::StreamExt;
use log;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::Utf8Bytes;

pub struct WebSocketClientRunner {
    service: ProxyServiceHandle,
    tls: Option<TlsAcceptor>,
    stream: Option<TcpStream>,
}

impl WebSocketClientRunner {
    pub fn new(service: ProxyServiceHandle, tls: Option<TlsAcceptor>, stream: TcpStream) -> Self {
        WebSocketClientRunner {
            service,
            tls,
            stream: Some(stream),
        }
    }

    pub fn run(self) {
        tokio::spawn(async move {
            if let Err(err) = self.run_inner().await {
                log::error!("WebSocket error: {}", err);
            }
        });
    }

    async fn run_inner(mut self) -> anyhow::Result<()> {
        let stream = self.stream.take().unwrap();
        // Do TLS handshake if needed
        if self.tls.is_some() {
            let tls = self.tls.take().unwrap();
            let stream = tls.accept(stream).await?;
            self.run_client(stream).await
        } else {
            self.run_client(stream).await
        }
    }

    async fn run_client<S>(mut self, stream: S) -> anyhow::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let ws_stream = tokio_tungstenite::accept_async(stream).await?;
        let (mut writer, mut reader) = ws_stream.split();

        // Read loop
        while let Some(msg) = reader.next().await {
            let msg = msg?;
            match msg {
                Message::Text(utf8_bytes) => self.handle_command(utf8_bytes, &mut writer).await?,
                Message::Binary(bytes) => self.handle_datagram(bytes, &mut writer).await?,
                Message::Ping(bytes) => writer.send(Message::Pong(bytes)).await?,
                Message::Pong(_bytes) => {}
                Message::Close(close_frame) => match close_frame {
                    Some(frame) => {
                        anyhow::bail!("Received close frame {}: {}", frame.code, frame.reason)
                    }
                    None => anyhow::bail!("Received close frame"),
                },
                Message::Frame(_) => panic!("Unexpected frame"),
            }
        }

        Ok(())
    }

    async fn handle_command<S>(
        &mut self,
        raw: Utf8Bytes,
        writer: &mut SplitSink<WebSocketStream<S>, Message>,
    ) -> anyhow::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        // The command should be ASCII.
        if !raw.is_ascii() {
            anyhow::bail!("Command contains non-ascii characters");
        }
        if raw.starts_with("PING ") {
            // Change PING to PONG and reply the same message.
            let mut response: String = raw.to_string();
            response.replace_range(1..2, "O");
            writer
                .send(Message::Text(Utf8Bytes::from(&response)))
                .await?;
            return Ok(());
        }
        let tokens: Vec<&str> = raw.split_ascii_whitespace().take(6).collect();
        match tokens.iter().next() {
            Some(&"MAKEVPN") => {
                if tokens.len() != 2 {
                    anyhow::bail!("Bad args to MAKEVPN");
                }
                let game = tokens[1];
                let (server_code, client_code) = self.service.make_vpn(game).await;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn handle_datagram<S>(
        &mut self,
        bytes: Bytes,
        writer: &mut SplitSink<WebSocketStream<S>, Message>,
    ) -> anyhow::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        todo!()
    }
}
