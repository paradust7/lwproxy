use std::net::SocketAddr;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::service::lease::LeaseHandler;
use crate::service::lease::ProxyLease;

struct UdpProxyLease {
    transmit: UnboundedSender<Bytes>,
    join: JoinHandle<()>,
}

impl LeaseHandler for UdpProxyLease {
    fn send(&self, data: Bytes) -> anyhow::Result<()> {
        self.transmit.send(data)?;
        Ok(())
    }

    fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn is_live(&self) -> bool {
        !self.transmit.is_closed()
    }
}

pub struct UdpProxy {
    addr: SocketAddr,
    relay_tx: Option<UnboundedSender<Bytes>>,
}

impl UdpProxy {
    pub fn new(addr: SocketAddr, relay_tx: UnboundedSender<Bytes>) -> Self {
        Self {
            addr,
            relay_tx: Some(relay_tx),
        }
    }

    pub async fn run(
        self,
        mut receiver: UnboundedReceiver<Bytes>,
        relay_tx: &mut UnboundedSender<Bytes>,
    ) -> anyhow::Result<()> {
        let remote_addr = self.addr;
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        let mut buf: [u8; 4096] = [0; 4096];
        loop {
            tokio::select! {
                r = socket.recv_from(&mut buf) => {
                    let (len, addr) = r?;
                    // Only accept packets from remote_addr
                    if addr == remote_addr {
                        relay_tx.send(Bytes::from(buf[..len].to_owned()))?;
                    }
                }
                r = receiver.recv() => {
                    if let Some(buf) = r {
                        socket.send_to(&buf, remote_addr).await?;
                    } else {
                        // Hangup
                        return Ok(());
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
                    log::info!("UdpProxy error: {}", err);
                }
            }
        });
        ProxyLease::new(UdpProxyLease { transmit: tx, join })
    }
}
