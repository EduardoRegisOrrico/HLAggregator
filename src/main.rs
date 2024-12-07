use hl_aggregator::{
    AggregatorConfig,
    aggregator::DerivativesAggregator,
};
use anyhow::Result;
use tokio::time::{sleep, Duration};
use tokio::signal::ctrl_c;
use std::io::{self, Write, stdin};
use hl_aggregator::trading::{OrderType, TradeRequest};
use hl_aggregator::trading::hyperliquid_service::HyperliquidService;
use hl_aggregator::trading::wallet::WalletManager;

enum MenuOption {
    ViewDydx,
    ViewHyperliquid,
    ViewPositions,
    ChangeSymbol,
    PlaceTrade,
    ManageWallets,
    Exit,
}

impl MenuOption {
    fn from_str(input: &str) -> Option<Self> {
        match input.trim() {
            "1" => Some(Self::ViewDydx),
            "2" => Some(Self::ViewHyperliquid),
            "3" => Some(Self::ViewPositions),
            "4" => Some(Self::ChangeSymbol),
            "5" => Some(Self::PlaceTrade),
            "6" => Some(Self::ManageWallets),
            "7" => Some(Self::Exit),
            _ => None,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
                // print!("\x1B[2J\x1B[H"); // Clear screen
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
                println!("3. View Open Positions");
                println!("4. Change Symbol");
                println!("5. Place Trade");
                println!("6. Manage Wallets");
                println!("7. Exit");
                print!("\nEnter choice (1-7): ");
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
                Some(MenuOption::ViewPositions) => {
                    selected_exchange = Some("Positions".to_string());
                },
                Some(MenuOption::PlaceTrade) => {
                    if let Some(exchange) = &selected_exchange {
                        if exchange != "Positions" {
                            handle_trade(exchange, &symbol).await?;
                        } else {
                            println!("Please select an exchange orderbook first");
                        }
                    } else {
                        println!("Please select an exchange orderbook first");
                    }
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
                Some(MenuOption::ManageWallets) => {
                    let mut wallet_manager = WalletManager::new()?;
                    wallet_manager.manage_wallets().await?;
                },
                Some(MenuOption::Exit) => break,
                None => {}
            }
        }
    }

    // Initialize the service
    let wallet_manager = WalletManager::new()?;
    let trading_service = HyperliquidService::new(&wallet_manager).await?;

    // Example market buy order
    let market_order = TradeRequest {
        asset: "BTC".to_string(),
        order_type: OrderType::Market,
        is_buy: true,
        amount: 0.01,
        price: None,
        leverage: 5,  // 5x leverage
        reduce_only: false,
    };

    // Example limit sell order
    let limit_order = TradeRequest {
        asset: "BTC".to_string(),
        order_type: OrderType::Limit,
        is_buy: false,
        amount: 0.01,
        price: Some(45000.0),
        leverage: 3,  // 3x leverage
        reduce_only: false,
    };

    // Place the orders
    let market_result = trading_service.place_trade(market_order).await?;
    println!("Market order result: {:?}", market_result);

    let limit_result = trading_service.place_trade(limit_order).await?;
    println!("Limit order result: {:?}", limit_result);

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

async fn handle_trade(exchange: &str, symbol: &str) -> Result<(), Box<dyn std::error::Error>> {
    if exchange == "dYdX" {
        println!("dYdX trading not yet implemented");
        return Ok(());
    }

    let wallet_manager = WalletManager::new()?;
    let trading_service = HyperliquidService::new(&wallet_manager).await?;
    
    loop {
        println!("\nTrading {} on {}", symbol, exchange);
        println!("1. Market Buy");
        println!("2. Market Sell");
        println!("3. Limit Buy");
        println!("4. Limit Sell");
        println!("5. Back to Main Menu");
        print!("Select option (1-5): ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim() {
            "5" => return Ok(()),
            "1" | "2" | "3" | "4" => {
                let (order_type, is_buy) = match input.trim() {
                    "1" => (OrderType::Market, true),
                    "2" => (OrderType::Market, false),
                    "3" => (OrderType::Limit, true),
                    "4" => (OrderType::Limit, false),
                    _ => unreachable!(),
                };

                print!("Enter amount: ");
                io::stdout().flush()?;
                let mut amount = String::new();
                io::stdin().read_line(&mut amount)?;
                let amount: f64 = amount.trim().parse()?;

                print!("Enter leverage (1-100): ");
                io::stdout().flush()?;
                let mut leverage = String::new();
                io::stdin().read_line(&mut leverage)?;
                let leverage: u32 = leverage.trim().parse()?;

                let price = if matches!(order_type, OrderType::Limit) {
                    print!("Enter limit price: ");
                    io::stdout().flush()?;
                    let mut price = String::new();
                    io::stdin().read_line(&mut price)?;
                    Some(price.trim().parse::<f64>()?)
                } else {
                    None
                };

                let request = TradeRequest {
                    asset: symbol.to_string(),
                    order_type,
                    is_buy,
                    amount,
                    price,
                    leverage,
                    reduce_only: false,
                };

                match trading_service.place_trade(request).await {
                    Ok(response) => println!("Trade placed successfully: {:?}", response),
                    Err(e) => println!("Error placing trade: {}", e),
                }
            }
            _ => println!("Invalid option"),
        }
    }
}

