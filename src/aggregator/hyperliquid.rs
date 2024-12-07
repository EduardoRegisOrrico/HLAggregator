use hyperliquid_rust_sdk::{
    InfoClient,
    BaseUrl,
    Message,
    Subscription,
};
use tokio::{
    spawn,
    sync::mpsc::{unbounded_channel},
    sync::Mutex,
};
use std::collections::HashMap;
use chrono::Utc;
use super::types::{LeverageInfo, OrderBook, Level, MarketSummary};
use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;
use super::traits::ExchangeAggregator;
use serde::Deserialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AggregatorError {
    #[error("Asset not found: {0}")]
    AssetNotFound(String),
}

#[derive(Clone)]
pub struct HyperliquidAggregator {
    client: Arc<Mutex<InfoClient>>,
    subscription_ids: HashMap<String, u32>,
    current_symbol: Option<String>,
    current_orderbook: Arc<Mutex<Option<OrderBook>>>,
    current_summary: Arc<Mutex<Option<MarketSummary>>>,
    universe_cache: Arc<Mutex<Option<MetaResponse>>>,
}

impl std::fmt::Debug for HyperliquidAggregator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HyperliquidAggregator")
            .field("subscription_ids", &self.subscription_ids)
            .field("current_symbol", &self.current_symbol)
            .field("current_orderbook", &self.current_orderbook)
            .field("current_summary", &self.current_summary)
            .field("universe_cache", &self.universe_cache)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl ExchangeAggregator for HyperliquidAggregator {
    async fn new(testnet: bool) -> Result<Self> {
        let base_url = if testnet { BaseUrl::Testnet } else { BaseUrl::Mainnet };
        Ok(Self {
            client: Arc::new(Mutex::new(InfoClient::new(None, Some(base_url)).await?)),
            subscription_ids: HashMap::new(),
            current_symbol: None,
            current_orderbook: Arc::new(Mutex::new(None)),
            current_summary: Arc::new(Mutex::new(None)),
            universe_cache: Arc::new(Mutex::new(None)),
        })
    }

    async fn start_market_updates(&mut self, symbol: &str) -> Result<()> {
        self.current_symbol = Some(symbol.to_string());
        
        // Shared state for updates
        let orderbook = self.current_orderbook.clone();
        let summary = self.current_summary.clone();
        let symbol = symbol.to_string();
        let client = self.client.clone();

        spawn(async move {
            let mut consecutive_errors = 0;
            
            'connection_loop: loop {
                let (sender, mut receiver) = unbounded_channel();
                let result = client.lock().await.subscribe(
                    Subscription::L2Book {
                        coin: symbol.clone(),
                    },
                    sender,
                ).await;

                match result {
                    Ok(_) => {
                        consecutive_errors = 0;  // Reset error counter on successful connection
                        
                        while let Some(msg) = receiver.recv().await {
                            match msg {
                                Message::L2Book(book) => {
                                    let new_book = OrderBook {
                                        exchange: "Hyperliquid".to_string(),
                                        symbol: symbol.clone(),
                                        bids: convert_levels_from_book(&book.data.levels[0]),
                                        asks: convert_levels_from_book(&book.data.levels[1]),
                                        timestamp: Utc::now().timestamp_millis() as u64,
                                    };
                                    
                                    if !new_book.bids.is_empty() && !new_book.asks.is_empty() {
                                        *orderbook.lock().await = Some(new_book);
                                    }
                                }
                                _ => {
                                    // Just log unexpected message types, don't reconnect
                                    eprintln!("Hyperliquid websocket: Unexpected message type");
                                }
                            }
                        }
                        
                        // Channel closed normally - wait before reconnecting
                        eprintln!("Hyperliquid websocket channel closed, waiting before reconnection...");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                    Err(e) => {
                        consecutive_errors += 1;
                        eprintln!("Hyperliquid connection error (attempt {}): {}", consecutive_errors, e);
                        
                        // Implement exponential backoff
                        let wait_time = std::cmp::min(consecutive_errors * 5, 30);
                        tokio::time::sleep(std::time::Duration::from_secs(wait_time)).await;
                    }
                }
            }
        });

        Ok(())
    }

    async fn get_market_summary(&self, symbol: &str) -> Result<MarketSummary> {
        let client = reqwest::Client::new();
        let response = client.post("https://api.hyperliquid.xyz/info")
            .json(&serde_json::json!({
                "type": "metaAndAssetCtxs"
            }))
            .send()
            .await?;
        
        let response_text = response.text().await?;
        let data: Vec<serde_json::Value> = serde_json::from_str(&response_text)?;
        
        // Get the asset contexts from the second element of the array
        let asset_ctxs = &data[1];
        
        // Find the index of our symbol in the universe (first element)
        let universe: Vec<AssetMeta> = serde_json::from_value(data[0]["universe"].clone())?;
        let symbol_index = universe.iter()
            .position(|asset| asset.name == symbol)
            .ok_or_else(|| anyhow::anyhow!("Symbol not found"))?;
        
        // Get the corresponding asset context
        let asset_ctx: AssetContext = serde_json::from_value(asset_ctxs[symbol_index].clone())?;
        
        Ok(MarketSummary {
            symbol: symbol.to_string(),
            price: asset_ctx.mark_price.parse()?,
            volume_24h: asset_ctx.volume_24h.parse()?,
            open_interest: asset_ctx.open_interest.parse()?,
            funding_rate: asset_ctx.funding_rate.parse()?,
        })
    }

    async fn get_leverage_info(&self, symbol: &str) -> Result<LeverageInfo> {
        // Try to get from cache first
        let mut cache = self.universe_cache.lock().await;
        
        // If cache is empty or expired, fetch new data
        if cache.is_none() {
            let client = reqwest::Client::new();
            let response = client.post("https://api.hyperliquid.xyz/info")
                .json(&serde_json::json!({
                    "type": "meta"
                }))
                .send()
                .await?;
            
            let response_text = response.text().await?;
            let meta: MetaResponse = serde_json::from_str(&response_text)?;
            *cache = Some(meta);
        }

        // Find the asset in the universe
        let asset = cache.as_ref()
            .and_then(|meta| meta.universe.iter().find(|asset| asset.name == symbol))
            .ok_or_else(|| AggregatorError::AssetNotFound(format!("Symbol not found: {}", symbol)))?;

        Ok(LeverageInfo {
            exchange: "Hyperliquid".to_string(),
            symbol: symbol.to_string(),
            max_leverage: asset.max_leverage as f64,
        })
    }

    async fn get_orderbook(&self, symbol: &str) -> Result<OrderBook> {
        let l2_snapshot = self.client.lock().await.l2_snapshot(symbol.to_string()).await?;
        
        Ok(OrderBook {
            exchange: "Hyperliquid".to_string(),
            symbol: symbol.to_string(),
            bids: convert_levels(l2_snapshot.levels.get(0).map(|v| v.as_slice()).unwrap_or_default()),
            asks: convert_levels(l2_snapshot.levels.get(1).map(|v| v.as_slice()).unwrap_or_default()),
            timestamp: l2_snapshot.time,
        })
    }

    async fn get_available_assets(&self) -> Result<Vec<String>> {
        let meta = self.client.lock().await.meta().await?;
        Ok(meta.universe.iter()
            .map(|asset| asset.name.clone())
            .collect())
    }

    async fn is_testnet(&self) -> bool {
        matches!(
            self.client.lock().await.http_client.base_url.as_str(),
            "https://api.hyperliquid-testnet.xyz/api"
        )
    }
}

async fn get_market_summary(client: &InfoClient, symbol: &str) -> Result<MarketSummary> {
    let _meta = client.meta().await?;
    let all_mids = client.all_mids().await?;
    
    let price = all_mids.get(symbol)
        .ok_or_else(|| anyhow::anyhow!("Price not found"))?
        .parse()
        .map_err(|e| anyhow::anyhow!("Failed to parse price: {}", e))?;

    // Get funding rate (last 24h)
    let now = Utc::now().timestamp_millis() as u64;
    let day_ago = now - (24 * 60 * 60 * 1000);
    let funding = client.funding_history(symbol.to_string(), day_ago, Some(now)).await?;
    let funding_rate = funding.last()
        .map(|f| f.funding_rate.parse::<f64>().unwrap_or(0.0))
        .unwrap_or(0.0);

    // Calculate volume and open interest
    let recent_trades = client.recent_trades(symbol.to_string()).await?;
    let volume_24h = recent_trades.iter()
        .filter(|trade| trade.time >= day_ago)
        .map(|trade| {
            let price = trade.px.parse::<f64>().unwrap_or(0.0);
            let size = trade.sz.parse::<f64>().unwrap_or(0.0);
            price * size
        })
        .sum();

    Ok(MarketSummary {
        symbol: symbol.to_string(),
        price,
        volume_24h,
        open_interest: volume_24h, // Using volume as proxy
        funding_rate,
    })
}

fn convert_levels_from_book(levels: &Vec<hyperliquid_rust_sdk::BookLevel>) -> Vec<Level> {
    levels.iter()
        .map(|level| Level {
            price: level.px.parse().unwrap_or(0.0),
            size: level.sz.parse().unwrap_or(0.0),
            orders: level.n,
        })
        .collect()
}

fn convert_levels(levels: &[hyperliquid_rust_sdk::Level]) -> Vec<Level> {
    levels.iter()
        .map(|level| Level {
            price: level.px.parse().unwrap_or(0.0),
            size: level.sz.parse().unwrap_or(0.0),
            orders: level.n,
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct MetaResponse {
    universe: Vec<AssetMeta>,
}

#[derive(Debug, Deserialize)]
struct AssetMeta {
    name: String,
    #[serde(rename = "maxLeverage")]
    max_leverage: u32,
    #[serde(rename = "szDecimals")]
    sz_decimals: u8,
    #[serde(rename = "onlyIsolated", default)]
    only_isolated: bool,
}

#[derive(Debug, Deserialize)]
struct MetaAndAssetCtxsResponse {
    #[serde(flatten)]
    meta: MetaResponse,
    #[serde(flatten)]
    asset_ctxs: Vec<AssetContext>,
}

#[derive(Debug, Deserialize)]
struct AssetContext {
    #[serde(rename = "openInterest")]
    open_interest: String,
    #[serde(rename = "markPx")]
    mark_price: String,
    #[serde(rename = "funding")]
    funding_rate: String,
    #[serde(rename = "dayNtlVlm")]
    volume_24h: String,
}
