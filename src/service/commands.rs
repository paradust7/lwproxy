use crate::service::net::code::Passcode;
use crate::service::ProxyClientHandle;
use crate::service::ProxyServiceHandle;
use anyhow::Context;
use bytes::Bytes;
use std::net::IpAddr;
use std::net::SocketAddr;
use tokio::sync::mpsc::UnboundedSender;

use crate::service::lease::ProxyLease;

pub static MAX_COMMAND_SIZE: usize = 4096;

pub struct CommandProcessor<'a> {
    service: &'a ProxyServiceHandle,
    client: &'a ProxyClientHandle,
}

pub struct CommandResult {
    pub response: String,
    pub new_lease: Option<ProxyLease>,
    pub relay_udp: bool,
}

impl<'a> CommandProcessor<'a> {
    pub fn new(service: &'a ProxyServiceHandle, client: &'a ProxyClientHandle) -> Self {
        Self { service, client }
    }

    pub async fn handle_command(
        &self,
        raw: &[u8],
        relay_tx: &mut Option<UnboundedSender<Bytes>>,
    ) -> anyhow::Result<CommandResult> {
        // The command should be ASCII.
        if !raw.is_ascii() {
            anyhow::bail!("Command contains non-ascii characters");
        }
        let raw = str::from_utf8(raw)?;
        if raw.len() > 255 {
            anyhow::bail!("Command too long");
        }
        if let Some(ping_payload) = raw.strip_prefix("PING") {
            let response = "PONG".to_owned() + ping_payload;
            return Ok(CommandResult {
                response,
                new_lease: None,
                relay_udp: false,
            });
        }
        let tokens: Vec<&str> = raw.split_whitespace().take(6).collect();
        if tokens.len() < 1 {
            anyhow::bail!("Empty command");
        }
        let response: String;
        let mut new_lease: Option<ProxyLease> = None;
        let mut relay_udp = false;
        match tokens.iter().next() {
            Some(&"PROXY") => {
                if tokens.len() != 5 {
                    anyhow::bail!("Bad args to PROXY command");
                }
                let want_ipv6 = match tokens[1] {
                    "IPV4" => false,
                    "IPV6" => true,
                    _ => anyhow::bail!("Bad protocol in PROXY command"),
                };
                match tokens[2] {
                    "TCP" => {}
                    "UDP" => anyhow::bail!("UDP is carried on a bound link, not PROXY"),
                    _ => anyhow::bail!("Bad transport in PROXY command"),
                };
                let ip: IpAddr = tokens[3].parse().context("Bad address in PROXY command")?;
                anyhow::ensure!(
                    ip.is_ipv6() == want_ipv6,
                    "Address family does not match PROXY command"
                );
                let port: u16 = tokens[4].parse().context("Bad port in PROXY command")?;
                let addr: SocketAddr = SocketAddr::new(ip, port);
                if relay_tx.is_none() {
                    anyhow::bail!("PROXY command but relay is missing");
                }
                new_lease = Some(self.service.route(addr, relay_tx.take().unwrap()).await?);
                response = format!("PROXY OK");
            }
            Some(&"NEWADDR") => {
                if tokens.len() != 1 {
                    anyhow::bail!("Bad args to NEWADDR");
                }
                let (address, passcode) = self.service.assign().await;
                response = format!("ADDR {} {}", address, passcode);
            }
            Some(&"BIND") => {
                if relay_tx.is_none() {
                    anyhow::bail!("BIND command with missing relay");
                }
                if tokens.len() != 4 {
                    anyhow::bail!("Bad args to BIND");
                }
                let passcode = Passcode::parse(tokens[1]).context("Bad passcode in BIND")?;
                anyhow::ensure!(tokens[2] == "UDP", "Only UDP can be bound");
                let bind_port: u16 = tokens[3].parse().context("Bad port in BIND")?;
                let relay = relay_tx.as_ref().unwrap().clone();
                match self.service.bind(&passcode, bind_port, relay).await {
                    Ok(lease) => {
                        relay_tx.take();
                        new_lease = Some(lease);
                        relay_udp = true;
                        response = format!("BIND OK");
                    }
                    Err(err) => {
                        // An assignment that has expired, or that this proxy
                        // never made, is how a client learns to ask for a new
                        // address. It is answered, not fatal, and the relay is
                        // left in place so the client can try again here.
                        log::info!("C{}: BIND refused ({})", self.client.id(), err);
                        response = format!("BIND FAILED");
                    }
                }
            }
            _ => {
                anyhow::bail!("Unrecognized command: {}", tokens[0]);
            }
        };
        Ok(CommandResult {
            response,
            new_lease,
            relay_udp,
        })
    }
}
