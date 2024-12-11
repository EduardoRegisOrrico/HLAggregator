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
use super::hyperliquid::HyperliquidAggregator;
use dydx::indexer::{IndexerClient, OrdersMessage, Ticker, IndexerConfig, RestConfig, SockConfig};
use num_traits::ToPrimitive;

#[derive(Debug, Clone)]
pub struct DydxAggregator {
    ws_url: String,
    current_orderbook: Arc<Mutex<Option<OrderBook>>>,
    current_summary: Arc<Mutex<Option<MarketSummary>>>,
    current_leverage: Arc<Mutex<Option<LeverageInfo>>>,
    current_symbol: Option<String>,
    available_assets: Arc<Mutex<Vec<String>>>,
    hl_aggregator: Arc<HyperliquidAggregator>,
    feed_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
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
            feed_handle: Arc::new(Mutex::new(None)),
        })
    }

    async fn start_market_updates(&mut self, symbol: &str) -> Result<()> {
        // Cancel previous subscription if it exists
        if let Some(handle) = self.feed_handle.lock().await.take() {
            handle.abort();
        }

        let formatted_symbol = format!("{}-USD", symbol.to_uppercase());
        
        // Shared state
        let orderbook = self.current_orderbook.clone();
        let summary = self.current_summary.clone();
        let symbol_clone = symbol.to_string();

        let handle = spawn(async move {
            'connection_loop: loop {
                let config = IndexerConfig {
                    rest: RestConfig {
                        endpoint: "https://indexer.dydx.trade/".to_string(),
                    },
                    sock: SockConfig {
                        endpoint: "wss://indexer.dydx.trade/v4/ws".to_string(),
                        timeout: 1000,
                        rate_limit: std::num::NonZeroU32::new(2).unwrap(),
                    },
                };
                
                let mut client = IndexerClient::new(config);
                let ticker = Ticker(formatted_symbol.clone());
                
                match client.feed().orders(&ticker, false).await {
                    Ok(mut feed) => {
                        while let Some(message) = feed.recv().await {
                            match message {
                                OrdersMessage::Initial(initial) => {
                                    let mut asks = initial.contents.asks.into_iter()
                                        .map(|level| Level {
                                            price: level.price.0.to_f64().unwrap_or(0.0),
                                            size: level.size.0.to_f64().unwrap_or(0.0),
                                            orders: 1,
                                        })
                                        .collect::<Vec<Level>>();

                                    // Sort asks by price (lowest to highest)
                                    asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));

                                    // Take only the first 5 asks
                                    asks.truncate(5);

                                    let new_book = OrderBook {
                                        exchange: "dYdX".to_string(),
                                        symbol: symbol_clone.clone(),
                                        bids: initial.contents.bids.into_iter()
                                            .map(|level| Level {
                                                price: level.price.0.to_f64().unwrap_or(0.0),
                                                size: level.size.0.to_f64().unwrap_or(0.0),
                                                orders: 1,
                                            })
                                            .collect(),
                                        asks,
                                        timestamp: Utc::now().timestamp_millis() as u64,
                                    };
                                    *orderbook.lock().await = Some(new_book);
                                },
                                OrdersMessage::Update(update) => {
                                    if let Some(book) = orderbook.lock().await.as_mut() {
                                        // Update asks
                                        if let Some(asks) = update.contents.asks {
                                            for ask in asks {
                                                if ask.size.0.to_f64().unwrap_or(0.0) == 0.0 {
                                                    book.asks.retain(|a| a.price != ask.price.0.to_f64().unwrap_or(0.0));
                                                } else {
                                                    if let Some(existing) = book.asks.iter_mut().find(|a| a.price == ask.price.0.to_f64().unwrap_or(0.0)) {
                                                        existing.size = ask.size.0.to_f64().unwrap_or(0.0);
                                                    } else {
                                                        book.asks.push(Level {
                                                            price: ask.price.0.to_f64().unwrap_or(0.0),
                                                            size: ask.size.0.to_f64().unwrap_or(0.0),
                                                            orders: 1,
                                                        });
                                                    }
                                                }
                                            }
                                        }

                                        // Update bids
                                        if let Some(bids) = update.contents.bids {
                                            for bid in bids {
                                                if bid.size.0.to_f64().unwrap_or(0.0) == 0.0 {
                                                    book.bids.retain(|b| b.price != bid.price.0.to_f64().unwrap_or(0.0));
                                                } else {
                                                    if let Some(existing) = book.bids.iter_mut().find(|b| b.price == bid.price.0.to_f64().unwrap_or(0.0)) {
                                                        existing.size = bid.size.0.to_f64().unwrap_or(0.0);
                                                    } else {
                                                        book.bids.push(Level {
                                                            price: bid.price.0.to_f64().unwrap_or(0.0),
                                                            size: bid.size.0.to_f64().unwrap_or(0.0),
                                                            orders: 1,
                                                        });
                                                    }
                                                }
                                            }
                                        }

                                        // Sort and limit asks to 5 closest to market price
                                        book.asks.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));
                                        book.asks.truncate(10);

                                        // Sort bids highest to lowest
                                        book.bids.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
                                        book.bids.truncate(10);

                                        book.timestamp = Utc::now().timestamp_millis() as u64;
                                    }
                                }
                            }
                        }
                        
                        // Channel closed normally or subscription lost
                        //eprintln!("dYdX websocket channel closed, waiting before reconnection...");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    Err(e) => {
                        //eprintln!("dYdX connection error: {}. Waiting before retry...", e);
                        // Clear orderbook on subscription error
                        *orderbook.lock().await = None;
                        // Wait before retry to prevent rapid reconnection attempts
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        });

        *self.feed_handle.lock().await = Some(handle);
        self.current_symbol = Some(symbol.to_string());

        Ok(())
    }

    async fn get_market_summary(&self, symbol: &str) -> Result<MarketSummary> {
        let formatted_symbol = format!("{}-USD", symbol.to_uppercase());
        let config = IndexerConfig {
            rest: RestConfig {
                endpoint: "https://indexer.dydx.trade".to_string(),
            },
            sock: SockConfig {
                endpoint: "wss://indexer.dydx.trade/v4/ws".to_string(),
                timeout: 1000,
                rate_limit: std::num::NonZeroU32::new(2).unwrap(),
            },
        };
        
        let client = IndexerClient::new(config);
        let ticker = Ticker(formatted_symbol);
        
        // Get market data
        match client.markets().get_perpetual_market(&ticker).await {
            Ok(market) => {
                Ok(MarketSummary {
                    symbol: symbol.to_string(),
                    price: market.oracle_price.map(|p| p.0.to_f64().unwrap_or(0.0)).unwrap_or(0.0),
                    volume_24h: market.volume_24h.0.to_f64().unwrap_or(0.0),
                    open_interest: market.open_interest.to_f64().unwrap_or(0.0),
                    funding_rate: market.next_funding_rate.to_f64().unwrap_or(0.0),
                })
            },
            Err(e) => {
                log::error!("Failed to fetch market data for symbol: {}. Error: {:?}", symbol, e);
                Err(e.into())
            }
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