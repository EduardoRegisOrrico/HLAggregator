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
use types::{LeverageInfo, OrderBook, MarketSummary};
use std::io::Write;

#[derive(Debug, Clone)]
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
    pub exchanges: HashMap<String, Exchange>,
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
        print!("\x1B[u\x1B[J");
        
        println!("Aggregated Market Data for {} (Press Ctrl+C to exit)\n", symbol);
        
        // Get leverage info for each exchange
        let mut leverages = HashMap::new();
        for (name, exchange) in &self.exchanges {
            match exchange.get_leverage_info(symbol).await {
                Ok(leverage) => {
                    leverages.insert(name.clone(), leverage.max_leverage);
                }
                Err(e) => {
                    println!("Failed to get leverage for {}: {}", name, e);
                }
            }
        }
        
        // Market Summaries
        println!("{:<12} {:<14} {:<14} {:<16} {:>12}", 
            "Exchange", "Price", "Max Leverage", "Open Interest", "Funding Rate");
        println!("{:-<74}", "");
        
        for (name, exchange) in &self.exchanges {
            match exchange.get_market_summary(symbol).await {
                Ok(summary) => {
                    self.last_known_summaries.insert(name.clone(), summary.clone());
                    println!("{:<12} ${:<13.2} {:<13.0}x ${:<15.2} {:>11.4}%",
                        name,
                        summary.price,
                        leverages.get(name).unwrap_or(&20.0),
                        summary.open_interest,
                        summary.funding_rate * 100.0
                    );
                }
                Err(_) => {
                    if let Some(last_summary) = self.last_known_summaries.get(name) {
                        println!("{:<12} ${:<13.2} {:<13.0}x ${:<15.2} {:>11.4}%",
                            name,
                            last_summary.price,
                            leverages.get(name).unwrap_or(&20.0),
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
        // Update market summaries before displaying
        let mut futures = Vec::new();
        
        for (name, exchange) in &self.exchanges {
            let name = name.clone();
            let exchange = exchange.clone();
            let future = async move {
                if let Ok(summary) = exchange.get_market_summary(symbol).await {
                    Some((name, summary))
                } else {
                    None
                }
            };
            futures.push(future);
        }

        // Wait for all updates concurrently
        let results = futures::future::join_all(futures).await;
        for result in results {
            if let Some((name, summary)) = result {
                self.last_known_summaries.insert(name, summary);
            }
        }

        // Display the updated data
        println!("Market Summaries:");
        println!("{:<12} {:<14} {:<14} {:<16} {:>12}", 
            "Exchange", "Price", "Max Leverage", "Open Interest", "Funding Rate");
        println!("{:-<74}", "");
        
        for (name, summary) in &self.last_known_summaries {
            let leverage = if let Ok(info) = self.exchanges.get(name)
                .unwrap()
                .get_leverage_info(symbol).await {
                info.max_leverage
            } else {
                20.0
            };

            println!("{:<12} ${:<13.2}      {:<2}x        ${:<15.2} {:>11.4}%",
                name,
                summary.price,
                leverage,
                summary.open_interest,
                summary.funding_rate * 100.0
            );
        }
    }

    pub async fn display_exchange_orderbook(&self, exchange: &str, symbol: &str) {
        if let Some(exchange) = self.exchanges.get(exchange) {
            // Always fetch fresh orderbook data
            if let Ok(book) = exchange.get_orderbook(symbol).await {
                let market_price = if let Ok(summary) = exchange.get_market_summary(symbol).await {
                    summary.price
                } else {
                    0.0
                };

                println!("Asks:");
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

                println!("Bids:");
                // Display bids (highest to lowest)
                for bid in book.bids.iter().take(5) {
                    println!("\x1b[32m{:>10.4}     ${:>7.2}\x1b[0m",
                        bid.size,
                        bid.price
                    );
                }
            }
        }
    }

    pub async fn get_exchange_orderbook(&self, exchange: &str, symbol: &str) -> Result<OrderBook> {
        if let Some(exch) = self.exchanges.get(exchange) {
            exch.get_orderbook(symbol).await
        } else {
            Err(anyhow::anyhow!("Exchange not found"))
        }
    }

    pub async fn get_exchange_summary(&self, exchange: &str, symbol: &str) -> Result<MarketSummary> {
        if let Some(exch) = self.exchanges.get(exchange) {
            exch.get_market_summary(symbol).await
        } else {
            Err(anyhow::anyhow!("Exchange not found"))
        }
    }
}
