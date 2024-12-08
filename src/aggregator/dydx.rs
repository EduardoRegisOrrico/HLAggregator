use tokio_tungstenite::{
    connect_async,
    WebSocketStream,
    MaybeTlsStream,
    tungstenite::Message,
};
use tokio::net::TcpStream;
use serde::Deserialize;
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::Mutex;
use std::sync::Arc;
use chrono::Utc;
use tokio::time::Duration;
use super::traits::ExchangeAggregator;
use super::types::{OrderBook, MarketSummary, LeverageInfo, Level};
use tokio::spawn;
use crate::error::AggregatorError;
use std::collections::HashMap;
use super::hyperliquid::HyperliquidAggregator;

#[derive(Debug, Clone)]
pub struct DydxAggregator {
    ws_url: String,
    current_orderbook: Arc<Mutex<Option<OrderBook>>>,
    current_summary: Arc<Mutex<Option<MarketSummary>>>,
    current_leverage: Arc<Mutex<Option<LeverageInfo>>>,
    current_symbol: Option<String>,
    available_assets: Arc<Mutex<Vec<String>>>,
    hl_aggregator: Arc<HyperliquidAggregator>,
}

#[async_trait]
impl ExchangeAggregator for DydxAggregator {
    async fn new(testnet: bool) -> Result<Self> {
        let ws_url = if testnet {
            "wss://indexer.dydx.trade/v4/ws".to_string()
        } else {
            "wss://indexer.dydx.trade/v4/ws".to_string()
        };
        
        Ok(Self { 
            ws_url,
            current_orderbook: Arc::new(Mutex::new(None)),
            current_summary: Arc::new(Mutex::new(None)),
            current_leverage: Arc::new(Mutex::new(None)),
            current_symbol: None,
            available_assets: Arc::new(Mutex::new(Vec::new())),
            hl_aggregator: Arc::new(HyperliquidAggregator::new(testnet).await?),
        })
    }

    async fn start_market_updates(&mut self, symbol: &str) -> Result<()> {
        let (ws_stream, _) = connect_async(&self.ws_url).await?;
        let (mut write, mut read) = ws_stream.split();

        // Subscribe to markets channel (this stays constant)
        let markets_sub = json!({
            "type": "subscribe",
            "channel": "v4_markets",
        });
        write.send(Message::Text(markets_sub.to_string())).await?;

        // Initial subscription for the symbol
        self.update_subscription(&mut write, symbol).await?;
        
        // Shared state
        let orderbook = self.current_orderbook.clone();
        let summary = self.current_summary.clone();
        let symbol = symbol.to_string();
        let ws_url = self.ws_url.clone();

        spawn(async move {
            let mut consecutive_errors = 0;
            
            'connection_loop: loop {
                while let Some(msg) = read.next().await {
                    match msg {
                        Ok(Message::Text(text)) => {
                            consecutive_errors = 0;  // Reset error counter on successful message
                            
                            if let Ok(value) = serde_json::from_str::<Value>(&text) {
                                if let Some(msg_type) = value["type"].as_str() {
                                    match msg_type {
                                        "subscribed" | "channel_data" => {
                                            match value["channel"].as_str() {
                                                Some("v4_orderbook") => {
                                                    if let Some(contents) = value["contents"].as_object() {
                                                        if msg_type == "subscribed" {
                                                            update_orderbook_from_initial(&orderbook, contents, &symbol).await;
                                                        } else {
                                                            update_orderbook_from_update(&orderbook, contents, &symbol).await;
                                                        }
                                                    }
                                                },
                                                Some("v4_markets") => {
                                                    handle_markets_update(&value, &summary, &symbol).await;
                                                },
                                                _ => {}
                                            }
                                        },
                                        _ => {}
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("dYdX websocket error: {}", e);
                            consecutive_errors += 1;
                            
                            if consecutive_errors >= 3 {
                                // Attempt to reconnect
                                break 'connection_loop;
                            }
                        }
                        _ => {}
                    }
                }

                // Connection lost or loop broken, attempt to reconnect
                eprintln!("dYdX websocket connection lost, attempting to reconnect...");
                tokio::time::sleep(Duration::from_secs(1)).await;
                
                // Attempt to establish new connection
                if let Ok((new_ws_stream, _)) = connect_async(&ws_url).await {
                    let (new_write, new_read) = new_ws_stream.split();
                    write = new_write;
                    read = new_read;
                    
                    // Resubscribe to channels
                    if let Err(e) = write.send(Message::Text(markets_sub.to_string())).await {
                        eprintln!("Failed to resubscribe to markets: {}", e);
                        continue;
                    }
                    
                    let orderbook_sub = json!({
                        "type": "subscribe",
                        "channel": "v4_orderbook",
                        "id": format!("{}-USD", symbol)
                    });
                    
                    if let Err(e) = write.send(Message::Text(orderbook_sub.to_string())).await {
                        eprintln!("Failed to resubscribe to orderbook: {}", e);
                        continue;
                    }
                }
            }
        });

        Ok(())
    }

    async fn get_market_summary(&self, _symbol: &str) -> Result<MarketSummary> {
        if let Some(summary) = self.current_summary.lock().await.as_ref() {
            Ok(summary.clone())
        } else {
            Err(AggregatorError::MarketDataNotFound(
                "No market data available".to_string()
            ).into())
        }
    }

    // using same max_leverage as hyperliquid cause dydx doesnt have a way to fetch it, theyre usually the same
    async fn get_leverage_info(&self, symbol: &str) -> Result<LeverageInfo> {
        let hl_leverage = self.hl_aggregator.get_leverage_info(symbol).await?;
        
        Ok(LeverageInfo {
            exchange: "dYdX".to_string(),
            symbol: symbol.to_string(),
            max_leverage: hl_leverage.max_leverage,
        })
    }

    async fn get_orderbook(&self, _symbol: &str) -> Result<OrderBook> {
        if let Some(book) = self.current_orderbook.lock().await.as_ref() {
            Ok(book.clone())
        } else {
            Err(AggregatorError::MarketDataNotFound(
                "No orderbook data available".to_string()
            ).into())
        }
    }

    async fn get_available_assets(&self) -> Result<Vec<String>> {
        let assets = self.available_assets.lock().await;
        if assets.is_empty() {
            Err(AggregatorError::MarketDataNotFound(
                "No available assets data yet".to_string()
            ).into())
        } else {
            Ok(assets.clone())
        }
    }

    async fn is_testnet(&self) -> bool {
        // Check if the WebSocket URL contains testnet indicators
        self.ws_url.contains("testnet") || self.ws_url.contains("stage")
    }
}

// Helper methods
impl DydxAggregator {
    async fn get_current_price(&self, _symbol: &str) -> Result<f64> {
        if let Some(book) = &*self.current_orderbook.lock().await {
            // Calculate mid price from best bid and ask
            if let (Some(best_bid), Some(best_ask)) = (book.bids.first(), book.asks.first()) {
                return Ok((best_bid.price + best_ask.price) / 2.0);
            }
        }
        Err(anyhow::anyhow!("Price not available"))
    }

    async fn get_funding_rate(&self, symbol: &str) -> Result<f64> {
        let summary = self.current_summary.lock().await;
        match &*summary {
            Some(summary) if summary.symbol == symbol => {
                Ok(summary.funding_rate)
            },
            Some(_) => {
                Err(AggregatorError::MarketDataNotFound(
                    format!("Funding rate not found for symbol: {}", symbol)
                ).into())
            },
            None => {
                Err(AggregatorError::MarketDataNotFound(
                    "No market data available".to_string()
                ).into())
            }
        }
    }

    // Add a new method to handle subscription changes
    async fn update_subscription(
        &mut self, 
        write: &mut futures::stream::SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>, 
        symbol: &str
    ) -> Result<()> {
        // First unsubscribe from existing orderbook if any
        if let Some(old_symbol) = &self.current_symbol {
            let unsub_msg = json!({
                "type": "unsubscribe",
                "channel": "v4_orderbook",
                "id": format!("{}-USD", old_symbol)
            });
            write.send(Message::Text(unsub_msg.to_string())).await?;
        }

        // Clear current orderbook
        *self.current_orderbook.lock().await = None;

        // Subscribe to new symbol
        let sub_msg = json!({
            "type": "subscribe",
            "channel": "v4_orderbook",
            "id": format!("{}-USD", symbol)
        });
        write.send(Message::Text(sub_msg.to_string())).await?;

        self.current_symbol = Some(symbol.to_string());
        Ok(())
    }
}

async fn update_orderbook_from_initial(
    orderbook: &Arc<Mutex<Option<OrderBook>>>, 
    contents: &serde_json::Map<String, Value>,
    symbol: &str
) {
    let mut new_book = OrderBook {
        exchange: "dYdX".to_string(),
        symbol: symbol.to_string(),
        bids: Vec::new(),
        asks: Vec::new(),
        timestamp: Utc::now().timestamp_millis() as u64,
    };

    // Parse all orders first
    let mut all_bids: Vec<Level> = Vec::new();
    let mut all_asks: Vec<Level> = Vec::new();

    if let Some(bids) = contents.get("bids").and_then(|v| v.as_array()) {
        all_bids = bids.iter()
            .filter_map(|bid| {
                Some(Level {
                    price: bid.get("price")?.as_str()?.parse().ok()?,
                    size: bid.get("size")?.as_str()?.parse().ok()?,
                    orders: 1,
                })
            })
            .collect();
    }

    if let Some(asks) = contents.get("asks").and_then(|v| v.as_array()) {
        all_asks = asks.iter()
            .filter_map(|ask| {
                Some(Level {
                    price: ask.get("price")?.as_str()?.parse().ok()?,
                    size: ask.get("size")?.as_str()?.parse().ok()?,
                    orders: 1,
                })
            })
            .collect();
    }

    // Sort without limiting
    all_bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap());
    all_asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap());

    // Store all orders
    new_book.bids = all_bids;
    new_book.asks = all_asks;

    *orderbook.lock().await = Some(new_book);
}

async fn update_orderbook_from_update(
    orderbook: &Arc<Mutex<Option<OrderBook>>>, 
    contents: &serde_json::Map<String, Value>,
    symbol: &str
) {
    let mut book = orderbook.lock().await;
    if let Some(book) = book.as_mut() {
        let mut all_bids = book.bids.clone();
        let mut all_asks = book.asks.clone();

        // Update bids
        if let Some(bids) = contents.get("bids").and_then(|v| v.as_array()) {
            for bid in bids {
                if let Some(bid_arr) = bid.as_array() {
                    if bid_arr.len() == 2 {
                        let price: f64 = bid_arr[0].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                        let size: f64 = bid_arr[1].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                        
                        if size == 0.0 {
                            all_bids.retain(|b| b.price != price);
                        } else {
                            if let Some(existing) = all_bids.iter_mut().find(|b| b.price == price) {
                                existing.size = size;
                            } else {
                                all_bids.push(Level { price, size, orders: 1 });
                            }
                        }
                    }
                }
            }
        }

        // Update asks similarly
        if let Some(asks) = contents.get("asks").and_then(|v| v.as_array()) {
            for ask in asks {
                if let Some(ask_arr) = ask.as_array() {
                    if ask_arr.len() == 2 {
                        let price: f64 = ask_arr[0].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                        let size: f64 = ask_arr[1].as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                        
                        if size == 0.0 {
                            all_asks.retain(|a| a.price != price);
                        } else {
                            if let Some(existing) = all_asks.iter_mut().find(|a| a.price == price) {
                                existing.size = size;
                            } else {
                                all_asks.push(Level { price, size, orders: 1 });
                            }
                        }
                    }
                }
            }
        }

        // Simply sort without filtering or limiting
        all_bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap()); // Highest to lowest
        all_asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap()); // Lowest to highest

        // Store all orders
        book.bids = all_bids;
        book.asks = all_asks;
        book.timestamp = Utc::now().timestamp_millis() as u64;
    }
}

async fn handle_markets_update(value: &Value, summary: &Arc<Mutex<Option<MarketSummary>>>, symbol: &str) {
    if let Some(contents) = value["contents"].as_object() {
        if let Some(market) = contents.get("markets").and_then(|m| m.as_object()) {
            let market_key = format!("{}-USD", symbol);
            if let Some(market_data) = market.get(&market_key) {
                let new_summary = MarketSummary {
                    symbol: symbol.to_string(),
                    price: market_data["oraclePrice"].as_str()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0),
                    volume_24h: market_data["volume24H"].as_str()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0),
                    open_interest: market_data["openInterest"].as_str()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0),
                    funding_rate: market_data["nextFundingRate"].as_str()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0),
                };
                *summary.lock().await = Some(new_summary);
            }
        }
    }
}
