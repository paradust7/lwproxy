use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use chrono::DateTime;
use chrono::Utc;
use tokio::sync::mpsc;

use super::client::VpnClient;
use super::client::VpnClientLease;
use super::packet::VpnPacket;

pub struct Vpn {
    server_code: String,
    client_code: String,
    game: String,
    created: DateTime<Utc>,
    clients: HashMap<IpAddr, Arc<VpnClient>>,
    last_packet: DateTime<Utc>,
}

impl Vpn {
    pub fn new(server_code: &str, client_code: &str, game: &str) -> Self {
        Self {
            server_code: server_code.to_owned(),
            client_code: client_code.to_owned(),
            game: game.to_owned(),
            created: Utc::now(),
            clients: HashMap::new(),
            last_packet: Utc::now(),
        }
    }

    pub fn add_client(&mut self, code: &str, relay: mpsc::Sender<VpnPacket>) -> VpnClientLease {
        todo!()
    }
}
