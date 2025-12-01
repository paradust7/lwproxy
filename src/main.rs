use std::path;

use clap::Parser;

mod service;
mod settings;
mod websocket;
mod webtransport;

use tokio::task::JoinHandle;
use websocket::listener::WebSocketProxyListener;

use crate::service::ProxyService;
use crate::settings::Settings;
use crate::webtransport::listener::WebTransportProxyListener;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long, default_value = "settings.toml")]
    pub config: path::PathBuf,
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
                let certs = serve.read_certs()?;
                let mut listener =
                    WebSocketProxyListener::new_secure(service.clone(), serve.bind_address, certs)
                        .await?;
                join_handles.push(listener.start().await?);
            }
            "wt" => {
                let certs = serve.read_certs()?;
                let mut listener =
                    WebTransportProxyListener::new(service.clone(), serve.bind_address, certs)
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
