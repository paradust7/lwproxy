use crate::service::vpn::vpn::VpnCode;
use crate::service::ProxyClientHandle;
use crate::service::ProxyServiceHandle;
use anyhow::Context;
use bytes::Bytes;
use std::net::IpAddr;
use std::net::Ipv4Addr;
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
                if tokens[1] != "IPV4" {
                    anyhow::bail!("Bad protocol in PROXY command")
                }
                let is_udp = match tokens[2] {
                    "TCP" => false,
                    "UDP" => true,
                    _ => anyhow::bail!("Bad transport in PROXY command"),
                };
                let ip: Ipv4Addr = tokens[3].parse().context("Bad address in PROXY command")?;
                let port: u16 = tokens[4].parse().context("Bad port in PROXY command")?;
                let addr: SocketAddr = SocketAddr::new(IpAddr::V4(ip), port);
                if relay_tx.is_none() {
                    anyhow::bail!("PROXY command but relay is missing");
                }
                new_lease = Some(
                    self.service
                        .route(addr, is_udp, relay_tx.take().unwrap())
                        .await?,
                );
                relay_udp = is_udp;
                if new_lease.is_some() {
                    response = format!("PROXY OK");
                } else {
                    response = format!("PROXY FAILED");
                }
            }
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
                if relay_tx.is_none() {
                    anyhow::bail!("VPN command with missing relay");
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

                new_lease = Some(
                    self.service
                        .vpn_route(&code, bind_port, relay_tx.take().unwrap())
                        .await?,
                );
                relay_udp = true;
                if new_lease.is_some() {
                    response = format!("BIND OK");
                } else {
                    response = format!("BIND FAILED");
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
