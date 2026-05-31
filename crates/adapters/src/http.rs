//! HTTP adapter backed by `reqwest`.
use async_trait::async_trait;

use mlt_core::ports::{HttpPort, HttpRequest, HttpResponse, PortError};

/// `HttpPort` implemented with a shared `reqwest::Client` (connection-pooled).
#[derive(Debug, Clone)]
pub struct ReqwestHttp {
    client: reqwest::Client,
}

impl ReqwestHttp {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

impl Default for ReqwestHttp {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HttpPort for ReqwestHttp {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        let method = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|e| PortError::Io(format!("bad method: {e}")))?;
        let mut builder = self.client.request(method, &req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k, v);
        }
        if let Some(body) = req.body {
            builder = builder.body(body);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| PortError::Io(e.to_string()))?;
        let status = resp.status().as_u16();
        let body = resp
            .bytes()
            .await
            .map_err(|e| PortError::Io(e.to_string()))?
            .to_vec();
        Ok(HttpResponse { status, body })
    }
}
