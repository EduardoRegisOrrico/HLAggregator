#[derive(Debug, thiserror::Error)]
pub enum AggregatorError {
    #[error("Asset not found: {0}")]
    AssetNotFound(String),
    
    #[error("Market data not found: {0}")]
    MarketDataNotFound(String),
    
    #[error("Exchange error: {0}")]
    ExchangeError(String),
    
    #[error("API error: {0}")]
    ApiError(String),
    
    #[error("Websocket error: {0}")]
    WebsocketError(String),
} 