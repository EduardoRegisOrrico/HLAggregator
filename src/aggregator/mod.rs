pub mod types;
pub mod traits;
pub mod hyperliquid;
pub mod dydx;
pub mod websocket;

use anyhow::Result;
use std::collections::HashMap;
use crate::config::AggregatorConfig;
use async_trait::async_trait;
use traits::ExchangeAggregator;
use dydx::DydxAggregator;
use hyperliquid::HyperliquidAggregator;
use types::{LeverageInfo, OrderBook, MarketSummary, Level};
use std::io::Write;
use std::future::Future;

#[derive(Debug)]
pub enum Exchange {
    Dydx(DydxAggregator),
    Hyperliquid(HyperliquidAggregator),
}

#[async_trait]
impl ExchangeAggregator for Exchange {
    async fn new(testnet: bool) -> Result<Self> {
        unimplemented!("Use specific constructor")
    }

    async fn start_market_updates(&mut self, symbol: &str) -> Result<()> {
        match self {
            Exchange::Dydx(e) => e.start_market_updates(symbol).await,
            Exchange::Hyperliquid(e) => e.start_market_updates(symbol).await,
        }
    }

    async fn display_market_data(&self) {
        match self {
            Exchange::Dydx(e) => e.display_market_data().await,
            Exchange::Hyperliquid(e) => e.display_market_data().await,
        }
    }

    async fn get_market_summary(&self, symbol: &str) -> Result<MarketSummary> {
        match self {
            Exchange::Dydx(e) => e.get_market_summary(symbol).await,
            Exchange::Hyperliquid(e) => e.get_market_summary(symbol).await,
        }
    }

    async fn get_leverage_info(&self, symbol: &str) -> Result<LeverageInfo> {
        match self {
            Exchange::Dydx(e) => e.get_leverage_info(symbol).await,
            Exchange::Hyperliquid(e) => e.get_leverage_info(symbol).await,
        }
    }

    async fn get_orderbook(&self, symbol: &str) -> Result<OrderBook> {
        match self {
            Exchange::Dydx(e) => e.get_orderbook(symbol).await,
            Exchange::Hyperliquid(e) => e.get_orderbook(symbol).await,
        }
    }

    async fn get_available_assets(&self) -> Result<Vec<String>> {
        match self {
            Exchange::Dydx(e) => e.get_available_assets().await,
            Exchange::Hyperliquid(e) => e.get_available_assets().await,
        }
    }

    async fn is_testnet(&self) -> bool {
        match self {
            Exchange::Dydx(e) => e.is_testnet().await,
            Exchange::Hyperliquid(e) => e.is_testnet().await,
        }
    }
}

pub struct DerivativesAggregator {
    config: AggregatorConfig,
    exchanges: HashMap<String, Exchange>,
    last_known_summaries: HashMap<String, types::MarketSummary>,
}

impl DerivativesAggregator {
    pub async fn new(config: AggregatorConfig) -> Result<Self> {
        let mut exchanges = HashMap::new();
        
        exchanges.insert(
            "dYdX".to_string(),
            Exchange::Dydx(DydxAggregator::new(config.testnet).await?)
        );
        
        exchanges.insert(
            "Hyperliquid".to_string(),
            Exchange::Hyperliquid(HyperliquidAggregator::new(config.testnet).await?)
        );
        
        Ok(Self { 
            config, 
            exchanges,
            last_known_summaries: HashMap::new(),
        })
    }

    pub async fn display_aggregated_data(&mut self, symbol: &str) {
        // Clear screen from saved position
        print!("\x1B[u\x1B[J");
        
        println!("Aggregated Market Data for {} (Press Ctrl+C to exit)\n", symbol);
        
        // Market Summaries
        println!("Market Summaries:");
        println!("{:<12} {:<14} {:<16} {:<16} {:>12}", 
            "Exchange", "Price", "24h Volume", "Open Interest", "Funding Rate");
        println!("{:-<74}", "");
        
        for (name, exchange) in &self.exchanges {
            match exchange.get_market_summary(symbol).await {
                Ok(summary) => {
                    // Update last known good data
                    self.last_known_summaries.insert(name.clone(), summary.clone());
                    println!("{:<12} ${:<13.2} ${:<15.2} ${:<15.2} {:>11.4}%",
                        name,
                        summary.price,
                        summary.volume_24h,
                        summary.open_interest,
                        summary.funding_rate * 100.0
                    );
                }
                Err(_) => {
                    // Use last known good data if available
                    if let Some(last_summary) = self.last_known_summaries.get(name) {
                        println!("{:<12} ${:<13.2} ${:<15.2} ${:<15.2} {:>11.4}%",
                            name,
                            last_summary.price,
                            last_summary.volume_24h,
                            last_summary.open_interest,
                            last_summary.funding_rate * 100.0
                        );
                    }
                }
            }
        }
        
        // Update leverage info display
        println!("\nMax Leverage:");
        for (name, exchange) in &self.exchanges {
            if let Ok(leverage) = exchange.get_leverage_info(symbol).await {
                println!("{}: {}x", name, leverage.max_leverage);
            }
        }
        
        // Display orderbooks
        println!("\nOrderbook Comparison:");
        
        for (name, exchange) in &self.exchanges {
            match exchange.get_orderbook(symbol).await {
                Ok(book) => {
                    println!("\n{} Orderbook:", name);
                    println!("      Size          Price");
                    println!("{:-<30}", "");
                    
                    // Display asks in reverse order (highest to lowest)
                    for ask in book.asks.iter().rev().take(5) {
                        println!("\x1b[31m{:>10.4}     ${:>7.2}\x1b[0m",
                            ask.size,
                            ask.price
                        );
                    }
                    
                    // Show spread
                    if let (Some(lowest_ask), Some(highest_bid)) = (book.asks.first(), book.bids.first()) {
                        let spread = lowest_ask.price - highest_bid.price;
                        println!("{:-<30}", "");
                        println!("Spread: ${:.2}", spread);
                        println!("{:-<30}", "");
                    }
                    
                    // Display bids in order (highest to lowest)
                    for bid in book.bids.iter().take(5) {
                        println!("\x1b[32m{:>10.4}     ${:>7.2}\x1b[0m",
                            bid.size,
                            bid.price
                        );
                    }
                }
                Err(_) => {
                    // Keep last known data by not printing anything for errors
                    continue;
                }
            }
        }
        
        // Flush output
        let _ = std::io::stdout().flush();
    }

    pub async fn start_all_market_updates(&mut self, symbol: &str) -> Result<()> {
        for exchange in self.exchanges.values_mut() {
            if let Err(e) = exchange.start_market_updates(symbol).await {
                eprintln!("Failed to start updates for exchange: {}", e);
                // Continue with other exchanges even if one fails
                continue;
            }
        }
        Ok(())
    }

    pub async fn display_market_summaries(&mut self, symbol: &str) {
        println!("{:<12} {:<14} {:<16} {:<16} {:>12}", 
            "Exchange", "Price", "24h Volume", "Open Interest", "Funding Rate");
        println!("{:-<74}", "");
        
        for (name, exchange) in &self.exchanges {
            match exchange.get_market_summary(symbol).await {
                Ok(summary) => {
                    // Update last known good data
                    self.last_known_summaries.insert(name.clone(), summary.clone());
                    println!("{:<12} ${:<13.2} ${:<15.2} ${:<15.2} {:>11.4}%",
                        name,
                        summary.price,
                        summary.volume_24h,
                        summary.open_interest,
                        summary.funding_rate * 100.0
                    );
                }
                Err(_) => {
                    // Use last known good data if available
                    if let Some(last_summary) = self.last_known_summaries.get(name) {
                        println!("{:<12} ${:<13.2} ${:<15.2} ${:<15.2} {:>11.4}%",
                            name,
                            last_summary.price,
                            last_summary.volume_24h,
                            last_summary.open_interest,
                            last_summary.funding_rate * 100.0
                        );
                    }
                }
            }
        }
    }

    pub async fn display_exchange_orderbook(&self, exchange: &str, symbol: &str) {
        if let Some(exchange) = self.exchanges.get(exchange) {
            match exchange.get_orderbook(symbol).await {
                Ok(book) => {
                    let market_price = if let Ok(summary) = exchange.get_market_summary(symbol).await {
                        summary.price
                    } else {
                        0.0
                    };

                    println!("      Size          Price");
                    println!("{:-<30}", "");

                    // Get 5 closest asks (ascending)
                    println!("Asks:");
                    book.asks.iter()
                        .filter(|ask| ask.price > market_price)
                        .take(5)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .for_each(|ask| {
                            println!("\x1b[31m{:>10.4}     ${:>7.2}\x1b[0m",
                                ask.size,
                                ask.price
                            );
                        });

                    println!("{:-<30}", "");
                    println!("Market Price: ${:.2}", market_price);
                    println!("{:-<30}", "");

                    // Get 5 closest bids (descending)
                    println!("Bids:");
                    book.bids.iter()
                        .filter(|bid| bid.price < market_price)
                        .take(5)
                        .for_each(|bid| {
                            println!("\x1b[32m{:>10.4}     ${:>7.2}\x1b[0m",
                                bid.size,
                                bid.price
                            );
                        });
                }
                Err(e) => {
                    println!("Error fetching orderbook: {}", e);
                }
            }
        }
    }
}
