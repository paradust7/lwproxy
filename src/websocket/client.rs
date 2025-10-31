use crate::service::vpn::lease::VpnLease;
use crate::service::vpn::packet::VpnPacket;
use crate::service::vpn::packet::VpnPacketUdp;
use crate::service::vpn::vpn::VpnCode;
use crate::service::ProxyServiceHandle;
use anyhow::Context;
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
    wsid: u64,
    service: ProxyServiceHandle,
    tls: Option<TlsAcceptor>,
    stream: Option<TcpStream>,
    relay_tx: Option<UnboundedSender<VpnPacket>>,
    lease: Option<VpnLease>,
}

impl WebSocketClientRunner {
    pub fn new(
        wsid: u64,
        service: ProxyServiceHandle,
        tls: Option<TlsAcceptor>,
        stream: TcpStream,
    ) -> Self {
        WebSocketClientRunner {
            wsid,
            service,
            tls,
            stream: Some(stream),
            relay_tx: None,
            lease: None,
        }
    }

    pub fn run(self) {
        tokio::spawn(async move {
            let mut client = self;
            if let Err(err) = client.run_inner().await {
                log::error!("WS{}: WebSocket error: {}", client.wsid, err);
            }
        });
    }

    async fn run_inner(&mut self) -> anyhow::Result<()> {
        let mut stream = self.stream.take().unwrap();
        // Do TLS handshake if needed
        let err = if self.tls.is_some() {
            let tls = self.tls.take().unwrap();
            let mut tls_stream = tls.accept(&mut stream).await?;
            let err = self.run_client(&mut tls_stream).await;
            drop(tls_stream);
            err
        } else {
            let err = self.run_client(&mut stream).await;
            err
        };
        // TcpStream is forcibly closed here
        drop(stream);
        match err {
            Ok(()) => {
                log::info!("WS{}: Closed normally", self.wsid);
            }
            Err(err) => {
                log::info!("WS{}: Closed due to error: {}", self.wsid, err);
            }
        };
        Ok(())
    }

    async fn run_client<S>(&mut self, stream: &mut S) -> anyhow::Result<()>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let ws_stream = tokio_tungstenite::accept_async(stream).await?;
        let (mut writer, mut reader) = ws_stream.split();
        let (relay_tx, mut relay_rx) = tokio::sync::mpsc::unbounded_channel::<VpnPacket>();
        self.relay_tx = Some(relay_tx);

        tokio::select! {
            msg = reader.next() => {
                match msg {
                    Some(Ok(msg)) => {
                        let did_close = self.handle_message(msg, &mut writer).await?;
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
                    Some(packet) => {
                        match packet {
                            VpnPacket::VpnStop => panic!("This should never happen"),
                            VpnPacket::Udp(vpn_packet_udp) => {
                                writer.send(Message::Binary(vpn_packet_udp.encode1())).await?;
                            },
                        }
                    },
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

        Ok(())
    }

    async fn handle_message<S>(
        &mut self,
        msg: Message,
        writer: &mut SplitSink<WebSocketStream<S>, Message>,
    ) -> anyhow::Result<bool>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        match msg {
            Message::Text(utf8_bytes) => self.handle_command(utf8_bytes, writer).await?,
            Message::Binary(bytes) => self.handle_datagram(bytes).await?,
            Message::Ping(bytes) => writer.send(Message::Pong(bytes)).await?,
            Message::Pong(_bytes) => {}
            Message::Close(close_frame) => match close_frame {
                Some(frame) => {
                    writer.send(Message::Close(Some(frame.clone()))).await?;
                    log::info!(
                        "WS{}: Received close frame (code={}, reason={})",
                        self.wsid,
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
                    log::info!("WS{}: Received blank close frame", self.wsid,);
                    return Ok(true);
                }
            },
            Message::Frame(_) => panic!("Unexpected frame"),
        }
        Ok(false)
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
        if tokens.len() < 1 {
            anyhow::bail!("Empty command");
        }
        let response: String;
        match tokens.iter().next() {
            Some(&"MAKEVPN") => {
                if tokens.len() != 2 {
                    anyhow::bail!("Bad args to MAKEVPN");
                }
                let game = tokens[1];
                let vpn_config = self.service.make_vpn(game).await;
                response = format!(
                    "NEWVPN {} {}",
                    &vpn_config.server_code, &vpn_config.client_code
                );
            }
            Some(&"READVPN") => {
                if tokens.len() != 2 {
                    anyhow::bail!("Bad args to READVPN");
                }
                let hexcode = tokens[1];
                let vpn = self.service.get_vpn_info(hexcode).await;
                let game: &str = match &vpn {
                    Some(vpn) => &vpn.game,
                    None => &"_expired_",
                };
                response = format!("VPNINFO {}", game);
            }
            Some(&"VPN") => {
                if self.relay_tx.is_none() {
                    anyhow::bail!("VPN command with existing relay");
                }
                if tokens.len() != 6 {
                    anyhow::bail!("Invalid VPN command length");
                }
                let hexcode = tokens[1];
                anyhow::ensure!(tokens[2] == "BIND", "Invalid VPN action");
                anyhow::ensure!(tokens[3] == "IPV4", "Invalid VPN network type");
                anyhow::ensure!(tokens[4] == "UDP", "Invalid VPN transport layer");
                let bind_port: u16 = tokens[5].parse().context("Bind port parse")?;
                let code = VpnCode::from(hexcode).context("VpnCode parse error")?;

                let lease = self
                    .service
                    .vpn_route(&code, bind_port, self.relay_tx.take().unwrap());
                response = format!("BIND OK");
            }
            _ => {
                anyhow::bail!("Unrecognized command: {}", tokens[0]);
            }
        };
        writer
            .send(Message::Text(Utf8Bytes::from(response)))
            .await?;
        Ok(())
    }

    async fn handle_datagram(&mut self, bytes: Bytes) -> anyhow::Result<()> {
        match &self.lease {
            Some(lease) => {
                let mut packet = VpnPacketUdp::decode1(bytes)?;
                packet.from = lease.addr;
                lease.send(VpnPacket::Udp(packet))?;
                Ok(())
            }
            None => {
                anyhow::bail!("Datagram without target");
            }
        }
    }
}
