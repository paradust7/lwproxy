use std::net::IpAddr;

use anyhow::Context;
use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::service::lease::LeaseHandler;
use crate::service::lease::ProxyLease;

/// Most addresses a client is given for one name. A client uses the first
/// and keeps the next as a fallback, so a long list is of no use to anyone.
static MAX_RECORDS: usize = 4;

/// Record tags in a reply.
static TAG_IPV4: u8 = 4;
static TAG_IPV6: u8 = 6;

/// Which families a client will accept.
///
/// A query is a hostname and one of these, and the reply is a 1 byte record
/// count followed by that many records of a 1 byte family tag and 4 or 16
/// bytes of address.
enum Filter {
    V4,
    V6,
    Any,
}

impl Filter {
    fn accepts(&self, addr: &IpAddr) -> bool {
        match self {
            Filter::V4 => addr.is_ipv4(),
            Filter::V6 => addr.is_ipv6(),
            Filter::Any => true,
        }
    }
}

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
            relay_tx.send(Self::handle_query(r).await)?;
        }
        Ok(())
    }

    /// Answers one query. A client is waiting on a reply, so a bad query or
    /// a failed lookup is answered with an empty record list rather than by
    /// dropping the connection.
    async fn handle_query(raw: Bytes) -> Bytes {
        let (hostname, filter) = match Self::parse_query(&raw) {
            Ok(parsed) => parsed,
            Err(err) => {
                log::info!("Bad DNS query: {}", err);
                return Self::encode(&Filter::Any, &[]);
            }
        };
        let addrs = Self::do_lookup(hostname).await;
        Self::encode(&filter, &addrs)
    }

    fn parse_query(raw: &[u8]) -> anyhow::Result<(&str, Filter)> {
        if !raw.is_ascii() {
            anyhow::bail!("Non-ascii hostname");
        }
        let text = str::from_utf8(raw)?;
        let mut tokens = text.split_whitespace();
        let hostname = tokens.next().context("Empty DNS query")?;
        let filter = match tokens.next().context("DNS query has no type")? {
            "A" => Filter::V4,
            "AAAA" => Filter::V6,
            "ANY" => Filter::Any,
            other => anyhow::bail!("Unknown DNS query type '{}'", other),
        };
        anyhow::ensure!(tokens.next().is_none(), "Trailing data in DNS query");
        Ok((hostname, filter))
    }

    /// Resolves a hostname to every address it has, without duplicates.
    async fn do_lookup(hostname: &str) -> Vec<IpAddr> {
        let found = match tokio::net::lookup_host((hostname, 0)).await {
            Ok(found) => found,
            Err(err) => {
                log::info!("DNS lookup of {} failed: {}", hostname, err);
                return Vec::new();
            }
        };
        let mut addrs: Vec<IpAddr> = Vec::new();
        for addr in found {
            let ip = addr.ip();
            if !addrs.contains(&ip) {
                addrs.push(ip);
            }
        }
        addrs
    }

    fn encode(filter: &Filter, addrs: &[IpAddr]) -> Bytes {
        // IPv6 first, which is what a client should try first when it can
        // use either.
        let selected: Vec<&IpAddr> = addrs
            .iter()
            .filter(|addr| addr.is_ipv6() && filter.accepts(addr))
            .chain(
                addrs
                    .iter()
                    .filter(|addr| addr.is_ipv4() && filter.accepts(addr)),
            )
            .take(MAX_RECORDS)
            .collect();
        let mut buffer = BytesMut::with_capacity(1 + selected.len() * 17);
        buffer.put_u8(selected.len() as u8);
        for addr in selected {
            match addr {
                IpAddr::V4(v4) => {
                    buffer.put_u8(TAG_IPV4);
                    buffer.put_slice(&v4.octets());
                }
                IpAddr::V6(v6) => {
                    buffer.put_u8(TAG_IPV6);
                    buffer.put_slice(&v6.octets());
                }
            }
        }
        buffer.freeze()
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
