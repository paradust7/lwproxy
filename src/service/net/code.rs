use std::fmt;
use std::net::Ipv6Addr;

use rand::Rng;

/// Characters a passcode is made of.
const PASSCODE_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// Length of a passcode, in characters.
pub const PASSCODE_LEN: usize = 16;

/// The first group of every address on the private network.
const PREFIX: u16 = 0xfd00;

/// Addresses below this are reserved for the proxy's own services, so that a
/// client is never assigned one of them.
const RESERVED: u64 = 256;

/// The proxy's own address on the private network. Its services (the
/// resolver, the HTTP CONNECT proxy) answer here.
pub const PROXY_ADDRESS: Ipv6Addr = Ipv6Addr::new(PREFIX, 0, 0, 0, 0, 0, 0, 1);

/// True for an address on the private network. Such an address means
/// something only inside this proxy.
pub fn is_network_address(addr: &Ipv6Addr) -> bool {
    addr.segments()[0] == PREFIX
}

/// A fresh address for a client, with 64 random bits.
///
/// Addresses sit in fd00::/16, which is private (unique local) space. The
/// address is what a player shares to invite others, so it is also the only
/// thing keeping one game's traffic away from another's.
pub fn random_address() -> Ipv6Addr {
    let mut rng = rand::rngs::ThreadRng::default();
    loop {
        let low: u64 = rng.random();
        if low < RESERVED {
            continue;
        }
        let mut octets = [0u8; 16];
        octets[0..2].copy_from_slice(&PREFIX.to_be_bytes());
        octets[8..16].copy_from_slice(&low.to_be_bytes());
        return Ipv6Addr::from(octets);
    }
}

/// The secret that lets a client bind ports on the address it was given.
///
/// It is handed out once, with the address, and never leaves the Luanti
/// instance that received it. The address is public; this is not.
#[derive(Hash, PartialEq, Eq, Clone)]
pub struct Passcode {
    code: String,
}

impl Passcode {
    pub fn new() -> Self {
        let mut rng = rand::rngs::ThreadRng::default();
        let code = (0..PASSCODE_LEN)
            .map(|_| PASSCODE_ALPHABET[rng.random_range(0..PASSCODE_ALPHABET.len())] as char)
            .collect();
        Self { code }
    }

    pub fn parse(text: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(text.len() == PASSCODE_LEN, "Passcode has the wrong length");
        anyhow::ensure!(
            text.bytes().all(|b| PASSCODE_ALPHABET.contains(&b)),
            "Passcode has invalid characters"
        );
        Ok(Self {
            code: text.to_owned(),
        })
    }
}

impl fmt::Display for Passcode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.code)
    }
}
