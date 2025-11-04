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
use tokio::task::JoinHandle;
use websocket::listener::WebSocketProxyListener;

use crate::service::ProxyService;
use crate::settings::Settings;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "settings.toml")]
    pub config: path::PathBuf,
}

pub struct CertData {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

fn read_certs(tls_cert: &path::PathBuf, tls_key: &path::PathBuf) -> anyhow::Result<CertData> {
    // Read PEM certificate chain
    let chain = fs::File::open(tls_cert)
        .with_context(|| format!("failed to open {}", tls_cert.display()))?;
    let mut chain = io::BufReader::new(chain);

    let chain: Vec<CertificateDer> = rustls_pemfile::certs(&mut chain)
        .collect::<Result<_, _>>()
        .with_context(|| format!("failed to load certs from {}", tls_cert.display()))?;

    anyhow::ensure!(
        !chain.is_empty(),
        "certificate chain is empty in {}",
        tls_cert.display()
    );

    // Read PEM private key
    let keys =
        fs::File::open(tls_key).with_context(|| format!("failed to open {}", tls_key.display()))?;

    // Read PEM private key
    let key = rustls_pemfile::private_key(&mut io::BufReader::new(keys))
        .context("failed to read private key")?
        .context("empty private key")?;
    Ok(CertData { chain, key })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Enable info logging.
    let env = env_logger::Env::default().default_filter_or("info");
    env_logger::init_from_env(env);

    let args = Args::parse();
    Settings::load(args.config)?;

    // Prevents https://github.com/snapview/tokio-tungstenite/issues/336
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to initialize rustls crypto provider");

    let (service, service_join) = ProxyService::new().start();

    let settings = Settings::get();
    if settings.serve.is_empty() {
        anyhow::bail!("No service endpoints configured. Is config file missing?")
    }

    let mut join_handles: Vec<JoinHandle<()>> = Vec::new();

    for serve in settings.serve.iter() {
        match serve.protocol.as_str() {
            "ws" => {
                let mut listener =
                    WebSocketProxyListener::new(service.clone(), serve.bind_address).await?;
                join_handles.push(listener.start().await?);
            }
            "wss" => {
                if serve.tls_cert.is_none() || serve.tls_key.is_none() {
                    anyhow::bail!("Missing tls file(s) for wss server");
                }
                let certs = read_certs(
                    serve.tls_cert.as_ref().unwrap(),
                    serve.tls_key.as_ref().unwrap(),
                )?;
                let mut listener = WebSocketProxyListener::new_secure(
                    service.clone(),
                    serve.bind_address,
                    certs.chain,
                    certs.key,
                )
                .await?;
                join_handles.push(listener.start().await?);
            }
            _ => panic!(
                "Unsupported protocol '{}' in serve directive",
                serve.protocol
            ),
        }
    }

    // WebTransport listener
    /*
    let mut listener = WebTransportProxyListener::new(service.clone(), args.wt_addr, chain, key)?;
    let handle = tokio::spawn(async move {
        listener.run().await;
    });
    */
    service_join.await?;
    for join in join_handles {
        join.await?;
    }

    Ok(())
}
