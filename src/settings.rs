use anyhow::Context;
use config::Config;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::PrivateKeyDer;
use rustls::pki_types::PrivatePkcs8KeyDer;
use serde::Deserialize;
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path;
use std::sync::Arc;
use std::sync::OnceLock;

pub static SETTINGS: OnceLock<Arc<Settings>> = OnceLock::new();
pub static SELFCERT: OnceLock<TLSCert> = OnceLock::new();

#[derive(Debug, Deserialize)]
pub struct ServeConfig {
    pub protocol: String,
    pub bind_address: SocketAddr,
    pub tls_cert: Option<path::PathBuf>,
    pub tls_key: Option<path::PathBuf>,
    pub tls_self: Option<bool>,
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

pub struct TLSCert {
    pub chain: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

impl Clone for TLSCert {
    fn clone(&self) -> Self {
        TLSCert {
            chain: self.chain.clone(),
            key: self.key.clone_key(),
        }
    }
}

impl ServeConfig {
    pub fn read_certs(&self) -> anyhow::Result<TLSCert> {
        if self.tls_self.is_some_and(|b| b) {
            return Ok(SELFCERT
                .get_or_init(|| {
                    TLSCert::make_self_cert()
                        .context("Generating self-signed certificate")
                        .unwrap()
                })
                .clone());
        }
        let tls_cert = self
            .tls_cert
            .as_ref()
            .ok_or_else(|| anyhow::format_err!("tls_cert missing"))?;
        let tls_key = self
            .tls_key
            .as_ref()
            .ok_or_else(|| anyhow::format_err!("tls_key missing"))?;

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
        let keys = fs::File::open(tls_key)
            .with_context(|| format!("failed to open {}", tls_key.display()))?;

        // Read PEM private key
        let key = rustls_pemfile::private_key(&mut io::BufReader::new(keys))
            .context("failed to read private key")?
            .context("empty private key")?;
        Ok(TLSCert { chain, key })
    }
}

impl TLSCert {
    fn make_self_cert() -> anyhow::Result<Self> {
        log::info!("Generating self-signed certificate");
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
        let cert_der = CertificateDer::from(cert.cert);
        let key = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
        Ok(Self {
            chain: vec![cert_der],
            key: PrivateKeyDer::Pkcs8(key),
        })
    }
}
