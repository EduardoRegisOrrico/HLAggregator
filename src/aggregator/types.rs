use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LeverageInfo {
    pub exchange: String,
    pub symbol: String,
    pub max_leverage: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OrderBook {
    pub exchange: String,
    pub symbol: String,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
    pub timestamp: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Level {
    pub price: f64,
    pub size: f64,
    pub orders: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MarketSummary {
    pub symbol: String,
    pub price: f64,
    pub volume_24h: f64,
    pub open_interest: f64,
    pub funding_rate: f64,
}

#[derive(Debug, Default)]
pub struct MarketData {
    pub orderbook: Option<OrderBook>,
    pub summary: Option<MarketSummary>,
    pub last_update: Option<std::time::Instant>,
}
