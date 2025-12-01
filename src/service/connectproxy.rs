use anyhow::Context;
use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpStream;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::service::lease::LeaseHandler;
use crate::service::lease::ProxyLease;
use crate::settings::Settings;

pub static SUCCESS_REPLY: &[u8] =
    b"HTTP/1.0 200 Connection Established\r\nProxy-agent: Apache/2.4.41 (Ubuntu)\r\n\r\n";

struct ConnectProxyLease {
    transmit: UnboundedSender<Bytes>,
    join: JoinHandle<()>,
}

struct ConnectHeader {
    host: String,
    port: u16,
}

pub struct ConnectProxy {
    relay_tx: Option<UnboundedSender<Bytes>>,
}

impl ConnectProxy {
    pub fn new(relay_tx: UnboundedSender<Bytes>) -> Self {
        Self {
            relay_tx: Some(relay_tx),
        }
    }

    pub fn header_complete(buf: &BytesMut) -> bool {
        // Do not parse the header until we see \r\n\r\n
        buf.windows(4).position(|w| w == b"\r\n\r\n").is_some()
    }

    fn parse_header(buf: Bytes) -> anyhow::Result<(ConnectHeader, Bytes)> {
        // In HTTP, the client is allowed to send request data beyond the CONNECT header before it receives a response.
        // If some of it ended up in the buffer, chop it off.
        let header_ending = buf
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or(anyhow::anyhow!("Invalid header"))?;
        let header = buf.slice(0..header_ending);
        let remainder = buf.slice(header_ending + 4..);
        if !header.is_ascii() {
            anyhow::bail!("Non-ascii in CONNECT header");
        }
        let header = match str::from_utf8(&header) {
            Ok(b) => b,
            Err(err) => anyhow::bail!(err),
        };
        // We're looking for
        // CONNECT hostname:port HTTP/1.1
        let firstline = header
            .lines()
            .next()
            .ok_or(anyhow::anyhow!("Missing line for CONNECT"))?;
        let tokens: Vec<&str> = firstline.split_ascii_whitespace().collect();
        if tokens.len() != 3 || tokens[0] != "CONNECT" || tokens[2] != "HTTP/1.1" {
            anyhow::bail!("Invalid CONNECT line");
        }
        let hostport: Vec<&str> = tokens[1].split(':').collect();
        if hostport.len() != 2 {
            anyhow::bail!("Invalid CONNECT host and port");
        }
        let host = hostport[0].to_owned();
        let port: u16 = hostport[1].parse().context("Invalid CONNECT port")?;

        Ok((ConnectHeader { host, port }, remainder))
    }

    pub async fn run(
        self,
        mut receiver: UnboundedReceiver<Bytes>,
        relay: &mut UnboundedSender<Bytes>,
    ) -> anyhow::Result<()> {
        let mut buffer: Option<BytesMut> = Some(BytesMut::with_capacity(256));
        let mut conn_read: Option<OwnedReadHalf> = None;
        let mut conn_write: Option<OwnedWriteHalf> = None;
        let mut readbuf: Vec<u8> = vec![0; 8192];
        let settings = Settings::get();
        loop {
            tokio::select! {
                r = async { conn_read.as_mut().unwrap().read(&mut readbuf).await }, if conn_read.is_some() => {
                    let sz = r?;
                    // TODO: Is there a way to reuse buffers here?
                    relay.send(Bytes::from(readbuf[..sz].to_owned()))?;
                },
                r = receiver.recv() => {
                    match &r {
                        Some(bytes) => {
                            if let Some(iconn_write) = &mut conn_write {
                                iconn_write.write(&bytes).await?;
                            } else if let Some(mut buf) = buffer.take() {
                                buf.put_slice(bytes);
                                // Check to see if the command is full.
                                if Self::header_complete(&buf) {
                                    // Process the header and establish the link
                                    let (header, remainder) = Self::parse_header(buf.freeze())?;
                                    if !settings.http_proxy.allowed_hosts.contains(&header.host) {
                                        anyhow::bail!("Host not allowed in CONNECT proxy");
                                    }
                                    let desc: (&str, u16) = (&header.host, header.port);
                                    let iconn = TcpStream::connect(desc).await?;

                                    log::info!("ConnectProxy proxying to {}:{}", header.host, header.port);

                                    let (iconn_read, mut iconn_write) = iconn.into_split();
                                    if !remainder.is_empty() {
                                        iconn_write.write(&remainder).await?;
                                    }
                                    conn_read = Some(iconn_read);
                                    conn_write = Some(iconn_write);

                                    relay.send(Bytes::from(SUCCESS_REPLY))?;

                                } else {
                                    // Keep reading more of the header
                                    buffer = Some(buf);
                                }
                            }
                        },
                        None => {
                            anyhow::bail!("Client disconnected");
                        },
                    }
                }
            }
        }
    }

    pub fn into_lease(mut self) -> ProxyLease {
        let mut relay_tx = self.relay_tx.take().unwrap();
        let (tx, rx) = unbounded_channel();
        let join = tokio::spawn(async move {
            match self.run(rx, &mut relay_tx).await {
                Ok(()) => {}
                Err(err) => {
                    // rx has been dropped, so the lease is no longer alive.
                    // This should wakeup the client, allowing it to notice.
                    let _ = relay_tx.send(Bytes::new());
                    log::info!("ConnectProxy error: {}", err);
                }
            }
        });
        ProxyLease::new(ConnectProxyLease { transmit: tx, join })
    }
}

impl LeaseHandler for ConnectProxyLease {
    fn send(&self, data: bytes::Bytes) -> anyhow::Result<()> {
        self.transmit.send(data)?;
        Ok(())
    }

    fn close(&self) -> anyhow::Result<()> {
        // Dropping transmit will trigger disconnect
        Ok(())
    }

    fn is_live(&self) -> bool {
        !self.transmit.is_closed()
    }
}
