use tokio_tungstenite::{connect_async, WebSocketStream};
use anyhow::Result;

pub struct WebSocketClient {
    url: String,
}

impl WebSocketClient {
    pub fn new(url: &str) -> Self {
        Self {
            url: url.to_string(),
        }
    }

    pub async fn connect(&self) -> Result<WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>> {
        let (ws_stream, _) = connect_async(&self.url).await?;
        Ok(ws_stream)
    }
}
