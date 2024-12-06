use hl_aggregator::{
    AggregatorConfig,
    aggregator::DerivativesAggregator,
};
use anyhow::Result;
use tokio::time::{sleep, Duration};
use tokio::signal::ctrl_c;
use std::io::{self, Write, stdin};

enum MenuOption {
    ViewDydx,
    ViewHyperliquid,
    ChangeSymbol,
    Exit,
}

impl MenuOption {
    fn from_str(input: &str) -> Option<Self> {
        match input.trim() {
            "1" => Some(Self::ViewDydx),
            "2" => Some(Self::ViewHyperliquid),
            "3" => Some(Self::ChangeSymbol),
            "4" => Some(Self::Exit),
            _ => None,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let mut symbol = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "BTC".to_string())
        .to_uppercase();

    // Create aggregator with default config
    let config = AggregatorConfig::default();
    let mut aggregator = DerivativesAggregator::new(config).await?;
    
    // Start initial market data updates
    start_market_updates(&mut aggregator, &symbol).await?;

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
    
    // Handle Ctrl+C
    tokio::spawn(async move {
        if let Ok(()) = ctrl_c().await {
            let _ = shutdown_tx.send(());
        }
    });

    let mut selected_exchange: Option<String> = None;
    let mut last_update = tokio::time::Instant::now();
    let refresh_interval = tokio::time::Duration::from_millis(1000);

    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(last_update + refresh_interval) => {
                print!("\x1B[2J\x1B[H"); // Clear screen
                println!("Current Symbol: {}", symbol);
                println!("\nMarket Summaries:");
                aggregator.display_market_summaries(&symbol).await;
                
                if let Some(exchange) = &selected_exchange {
                    println!("\nOrderbook for {}:", exchange);
                    aggregator.display_exchange_orderbook(exchange, &symbol).await;
                }

                println!("\nMenu Options:");
                println!("1. View dYdX Orderbook");
                println!("2. View Hyperliquid Orderbook");
                println!("3. Change Symbol");
                println!("4. Exit");
                print!("\nEnter choice (1-4): ");
                io::stdout().flush()?;

                last_update = tokio::time::Instant::now();
            }
            _ = &mut shutdown_rx => {
                println!("\nShutting down gracefully...");
                break;
            }
        }

        // Non-blocking input check
        if let Ok(input) = non_blocking_read_line() {
            match MenuOption::from_str(&input) {
                Some(MenuOption::ViewDydx) => {
                    selected_exchange = Some("dYdX".to_string());
                },
                Some(MenuOption::ViewHyperliquid) => {
                    selected_exchange = Some("Hyperliquid".to_string());
                },
                Some(MenuOption::ChangeSymbol) => {
                    print!("Enter new symbol: ");
                    io::stdout().flush()?;
                    if let Ok(new_symbol) = blocking_read_line() {
                        symbol = new_symbol.trim().to_uppercase();
                        selected_exchange = None;
                        start_market_updates(&mut aggregator, &symbol).await?;
                    }
                },
                Some(MenuOption::Exit) => break,
                None => {}
            }
        }
    }

    Ok(())
}

async fn start_market_updates(aggregator: &mut DerivativesAggregator, symbol: &str) -> Result<()> {
    aggregator.start_all_market_updates(symbol).await?;
    sleep(Duration::from_secs(2)).await; // Give time for initial data
    Ok(())
}

fn non_blocking_read_line() -> io::Result<String> {
    let mut input = String::new();
    if stdin().read_line(&mut input)? > 0 {
        Ok(input)
    } else {
        Err(io::Error::new(io::ErrorKind::WouldBlock, "No input available"))
    }
}

fn blocking_read_line() -> io::Result<String> {
    let mut input = String::new();
    stdin().read_line(&mut input)?;
    Ok(input)
}

