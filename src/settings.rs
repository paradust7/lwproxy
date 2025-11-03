use config::Config;
use serde::Deserialize;
use std::sync::Arc;
use std::sync::OnceLock;

pub static SETTINGS: OnceLock<Arc<Settings>> = OnceLock::new();

#[derive(Debug, Deserialize)]
pub struct Settings {
    // Hosts that the HTTP proxy will allow
    pub allowed_hosts: Vec<String>,
}

impl Settings {
    pub fn load() -> anyhow::Result<()> {
        let settings = Config::builder()
            .set_default("allowed_hosts", Vec::<String>::new())?
            .add_source(config::File::with_name("settings").required(true))
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
