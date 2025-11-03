use std::fs;
use std::io;
use std::path;

use anyhow::Context;

use clap::Parser;
use config::Config;
use rustls::pki_types::CertificateDer;

mod service;
mod settings;
mod websocket;
mod webtransport;

use rustls::pki_types::PrivateKeyDer;
use websocket::listener::WebSocketProxyListener;

use crate::service::ProxyService;
use crate::settings::Settings;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// WebSocket (HTTP/1.1) bind address and port
    #[arg(long, default_value = "[::]:7777")]
    ws_addr: std::net::SocketAddr,

    /// Use HTTPS for websocket
    #[arg(long, default_value = "false")]
    ws_secure: bool,

    /// WebTransport (HTTP/3 + SSL over UDP) bind address and port
    #[arg(short, long, default_value = "[::]:1443")]
    wt_addr: std::net::SocketAddr,

    /// Use the certificates at this path, encoded as PEM.
    #[arg(long, default_value = None)]
    pub tls_cert: Option<path::PathBuf>,

    /// Use the private key at this path, encoded as PEM.
    #[arg(long, default_value = None)]
    pub tls_key: Option<path::PathBuf>,
}

pub struct CertData {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

fn read_certs(args: &Args) -> anyhow::Result<CertData> {
    // Read PEM certificate chain
    let chain =
        fs::File::open(args.tls_cert.as_ref().unwrap()).context("failed to open cert file")?;
    let mut chain = io::BufReader::new(chain);

    let chain: Vec<CertificateDer> = rustls_pemfile::certs(&mut chain)
        .collect::<Result<_, _>>()
        .context("failed to load certs")?;

    anyhow::ensure!(!chain.is_empty(), "could not find certificate");

    // Read PEM private key
    let keys = fs::File::open(args.tls_key.as_ref().unwrap()).context("failed to open key file")?;

    // Read PEM private key
    let key = rustls_pemfile::private_key(&mut io::BufReader::new(keys))
        .context("failed to load private key")?
        .context("missing private key")?;
    Ok(CertData { chain, key })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Enable info logging.
    let env = env_logger::Env::default().default_filter_or("info");
    env_logger::init_from_env(env);

    Settings::load()?;
    let args = Args::parse();

    // Prevents https://github.com/snapview/tokio-tungstenite/issues/336
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to initialize rustls crypto provider");

    let (service, service_join) = ProxyService::new().start();

    // WebSocket listener
    let mut listener = if args.ws_secure {
        let certs = read_certs(&args)?;
        WebSocketProxyListener::new_secure(
            service.clone(),
            args.ws_addr,
            certs.chain.clone(),
            certs.key.clone_key(),
        )
        .await?
    } else {
        WebSocketProxyListener::new(service.clone(), args.ws_addr).await?
    };
    let listener_join = listener.start().await?;

    // WebTransport listener
    /*
    let mut listener = WebTransportProxyListener::new(service.clone(), args.wt_addr, chain, key)?;
    let handle = tokio::spawn(async move {
        listener.run().await;
    });
    */
    service_join.await?;
    listener_join.await?;

    Ok(())
}
