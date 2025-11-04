use std::net::Ipv4Addr;

use bytes::Bytes;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::service::lease::LeaseHandler;
use crate::service::lease::ProxyLease;

static FAIL_ADDR: Ipv4Addr = Ipv4Addr::new(0, 0, 0, 0);

struct DNSProxyLease {
    transmit: UnboundedSender<Bytes>,
    join: JoinHandle<()>,
}

impl LeaseHandler for DNSProxyLease {
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
pub struct DNSProxy {
    relay_tx: Option<UnboundedSender<Bytes>>,
}

impl DNSProxy {
    pub fn new(relay_tx: UnboundedSender<Bytes>) -> Self {
        DNSProxy {
            relay_tx: Some(relay_tx),
        }
    }

    pub async fn run(
        self,
        mut receiver: UnboundedReceiver<Bytes>,
        relay_tx: &mut UnboundedSender<Bytes>,
    ) -> anyhow::Result<()> {
        while let Some(r) = receiver.recv().await {
            // All lookup failures are non-fatal and just result in 0.0.0.0 being returned.
            let response = match Self::do_lookup(r).await {
                Ok(r) => r,
                Err(err) => {
                    log::info!("DNS lookup failed: {}", err);
                    FAIL_ADDR
                }
            };
            let data: Vec<u8> = response.to_bits().to_be_bytes().into();
            relay_tx.send(Bytes::from(data))?;
        }
        Ok(())
    }

    pub async fn do_lookup(raw_hostname: Bytes) -> anyhow::Result<Ipv4Addr> {
        if !raw_hostname.is_ascii() {
            anyhow::bail!("Non-ascii hostname");
        }
        let hostname = str::from_utf8(&raw_hostname)?;
        for addr in tokio::net::lookup_host((hostname, 0)).await? {
            match addr {
                std::net::SocketAddr::V4(socket_addr_v4) => return Ok(socket_addr_v4.ip().clone()),
                std::net::SocketAddr::V6(_) => {}
            }
        }
        anyhow::bail!("Lookup failed")
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
                    log::info!("DNSProxy error: {}", err);
                }
            }
        });
        ProxyLease::new(DNSProxyLease { transmit: tx, join })
    }
}
