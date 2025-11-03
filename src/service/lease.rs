use bytes::Bytes;

pub trait LeaseHandler: Send {
    fn send(&self, data: Bytes) -> anyhow::Result<()>;
    fn close(&self) -> anyhow::Result<()>;
    fn is_live(&self) -> bool;
}
pub struct ProxyLease {
    handler: Box<dyn LeaseHandler>,
}

impl ProxyLease {
    pub fn new(handler: impl LeaseHandler + 'static) -> Self {
        Self {
            handler: Box::new(handler),
        }
    }

    pub fn send(&self, data: Bytes) -> anyhow::Result<()> {
        self.handler.send(data)
    }

    pub fn close(&self) -> anyhow::Result<()> {
        self.handler.close()
    }

    pub fn is_live(&self) -> bool {
        self.handler.is_live()
    }
}

impl Drop for ProxyLease {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
