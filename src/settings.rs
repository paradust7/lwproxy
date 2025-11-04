use config::Config;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path;
use std::sync::Arc;
use std::sync::OnceLock;

pub static SETTINGS: OnceLock<Arc<Settings>> = OnceLock::new();
pub static SUPPORTED_PROTOCOLS: [&str; 3] = ["ws", "wss", "wt"];

#[derive(Debug, Deserialize)]
pub struct ServeConfig {
    pub protocol: String,
    pub bind_address: SocketAddr,
    pub tls_cert: Option<path::PathBuf>,
    pub tls_key: Option<path::PathBuf>,
}

#[derive(Debug, Deserialize)]
pub struct HttpProxyConfig {
    pub allowed_hosts: Vec<String>,
}
#[derive(Debug, Deserialize)]
pub struct Settings {
    pub serve: Vec<ServeConfig>,
    // Hosts that the HTTP proxy will allow
    pub http_proxy: HttpProxyConfig,
}

impl Settings {
    pub fn load(path: path::PathBuf) -> anyhow::Result<()> {
        let settings = Config::builder()
            .add_source(config::File::from(path))
            .build()?;
        let settings: Settings = settings.try_deserialize()?;
        SETTINGS.get_or_init(|| Arc::new(settings));
        Ok(())
    }

    pub fn get() -> Arc<Settings> {
        match SETTINGS.get() {
            Some(settings) => settings.clone(),
            None => panic!("Settings uninitialized"),
        }
    }
}
