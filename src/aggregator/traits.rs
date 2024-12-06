use async_trait::async_trait;
use anyhow::Result;
use super::types::{LeverageInfo, OrderBook, MarketSummary};

#[async_trait]
pub trait ExchangeAggregator {
    async fn new(testnet: bool) -> Result<Self> where Self: Sized;
    async fn start_market_updates(&mut self, symbol: &str) -> Result<()>;
    async fn display_market_data(&self);
    async fn get_market_summary(&self, symbol: &str) -> Result<MarketSummary>;
    async fn get_leverage_info(&self, symbol: &str) -> Result<LeverageInfo>;
    async fn get_orderbook(&self, symbol: &str) -> Result<OrderBook>;
    async fn get_available_assets(&self) -> Result<Vec<String>>;
    async fn is_testnet(&self) -> bool;
}
