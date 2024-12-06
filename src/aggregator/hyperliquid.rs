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

pub struct HyperliquidAggregator {
    client: Arc<Mutex<InfoClient>>,
    subscription_ids: HashMap<String, u32>,
    current_symbol: Option<String>,
    current_orderbook: Arc<Mutex<Option<OrderBook>>>,
    current_summary: Arc<Mutex<Option<MarketSummary>>>,
}

impl std::fmt::Debug for HyperliquidAggregator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HyperliquidAggregator")
            .field("subscription_ids", &self.subscription_ids)
            .field("current_symbol", &self.current_symbol)
            .field("current_orderbook", &self.current_orderbook)
            .field("current_summary", &self.current_summary)
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

    async fn display_market_data(&self) {
        if let Some(ref symbol) = self.current_symbol {
            println!("\x1B[2J\x1B[1;1H"); // Clear screen
            println!("Market Data for {}\n", symbol);
            
            // Display orderbook
            if let Some(book) = &*self.current_orderbook.lock().await {
                println!("Orderbook:");
                println!("  Bids:");
                for level in book.bids.iter().take(5) {
                    println!("    ${}: {} {} ({} orders)", 
                        level.price, level.size, symbol, level.orders);
                }
                println!("  Asks:");
                for level in book.asks.iter().take(5) {
                    println!("    ${}: {} {} ({} orders)", 
                        level.price, level.size, symbol, level.orders);
                }
            }
            
            // Display market summary
            if let Some(summary) = &*self.current_summary.lock().await {
                println!("\nMarket Summary:");
                println!("  Price: ${:.2}", summary.price);
                println!("  24h Volume: ${:.2}", summary.volume_24h);
                println!("  Open Interest: ${:.2}", summary.open_interest);
                println!("  Funding Rate: {:.4}%", summary.funding_rate * 100.0);
            }
        }
    }

    async fn get_market_summary(&self, symbol: &str) -> Result<MarketSummary> {
        let meta = self.client.lock().await.meta().await?;
        let all_mids = self.client.lock().await.all_mids().await?;
        
        let price = all_mids.get(symbol)
            .ok_or_else(|| anyhow::anyhow!("Price not found"))?
            .parse()
            .map_err(|e| anyhow::anyhow!("Failed to parse price: {}", e))?;

        // Get funding rate (last 24h)
        let now = Utc::now().timestamp_millis() as u64;
        let day_ago = now - (24 * 60 * 60 * 1000);
        let funding = self.client.lock().await.funding_history(symbol.to_string(), day_ago, Some(now)).await?;
        let funding_rate = funding.last()
            .map(|f| f.funding_rate.parse::<f64>().unwrap_or(0.0))
            .unwrap_or(0.0);

        // Calculate 24h volume from recent trades
        let recent_trades = self.client.lock().await.recent_trades(symbol.to_string()).await?;
        let volume_24h = recent_trades.iter()
            .filter(|trade| trade.time >= day_ago)
            .map(|trade| {
                let price = trade.px.parse::<f64>().unwrap_or(0.0);
                let size = trade.sz.parse::<f64>().unwrap_or(0.0);
                price * size
            })
            .sum();

        // Get open interest from user state aggregation
        let _l2_snapshot = self.client.lock().await.l2_snapshot(symbol.to_string()).await?;
        let open_interest = meta.universe.iter()
            .find(|a| a.name == symbol)
            .map(|_| {
                // For now returning volume as a proxy since open interest 
                // requires additional API calls to aggregate user positions
                volume_24h
            })
            .unwrap_or(0.0);

        Ok(MarketSummary {
            symbol: symbol.to_string(),
            price,
            volume_24h,
            open_interest,
            funding_rate,
        })
    }

    async fn get_leverage_info(&self, symbol: &str) -> Result<LeverageInfo> {
        Ok(LeverageInfo {
            exchange: "Hyperliquid".to_string(),
            symbol: symbol.to_string(),
            max_leverage: 50.0,  // Fixed max leverage
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
