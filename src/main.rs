use hl_aggregator::{
    aggregator::{
        DerivativesAggregator, Exchange
    }, trading::wallet, AggregatorConfig
};
use hl_aggregator::aggregator::types::OrderBook;
use anyhow::Result;
use tokio::time::{sleep, Duration};
use std::io::{self, Write, Stdout};
use hl_aggregator::trading::{OrderType, TradeRequest};
use hl_aggregator::trading::hyperliquid_service::{HyperliquidService, OpenOrder};
use hl_aggregator::trading::wallet::WalletManager;
use hl_aggregator::aggregator::traits::ExchangeAggregator;
use ratatui::{
    backend::CrosstermBackend,
    widgets::{Block, Borders, Paragraph},
    layout::{Layout, Constraint, Direction, Rect},
    Terminal,
};
use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
    execute,
    cursor,
    terminal::{self, Clear, ClearType},
};
use hl_aggregator::aggregator::types::{MarketData, MarketSummary};
use env_logger;
use hl_aggregator::trading::positions::Position;
use ethers::signers::Signer;
use std::sync::Arc;
use tokio::sync::Mutex;
use dydx::indexer::types::{OrderSide, OrderType as DydxOrderType};
use dydx::node::OrderTimeInForce;
use hl_aggregator::trading::OrderType as LocalOrderType;
use hyperliquid_rust_sdk::ExchangeResponseStatus;
use chrono;
use env_logger::{Builder, Target};
use log::LevelFilter;
use tracing_subscriber::{fmt, EnvFilter};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use dydx::noble::NobleUsdc;
use bigdecimal::BigDecimal;
use std::str::FromStr;
use dydx::indexer::types::ApiOrderStatus;
use hl_aggregator::trading::orders::Order;
use dydx::indexer::OrderStatus;

pub fn init_logging() {
    let mut builder = Builder::from_default_env();
    builder
        .filter_module("dydx::indexer::sock::connector", LevelFilter::Off)
        .target(Target::Stdout)
        .init();
}

pub fn init_file_logging() {
    let file_appender = RollingFileAppender::new(
        Rotation::NEVER,
        "logs",
        "trading.log",
    );

    let subscriber = fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(file_appender)
        .with_ansi(false)
        .with_target(false)
        .with_thread_ids(true)
        .with_line_number(true)
        .with_file(true)
        .with_level(true)
        .compact()
        .try_init()
        .expect("Failed to set tracing subscriber");
}

enum MenuOption {
    ViewDydx,
    ViewHyperliquid,
    ViewPositions,
    ViewOpenOrders,
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
            "4" => Some(Self::ViewOpenOrders),
            "5" => Some(Self::ChangeSymbol),
            "6" => Some(Self::PlaceTrade),
            "7" => Some(Self::ManageWallets),
            "8" => Some(Self::Exit),
            _ => None,
        }
    }
}

struct App {
    aggregator: DerivativesAggregator,
    hyperliquid_service: HyperliquidService,
    wallet_manager: WalletManager,
    selected_exchange: Option<String>,
    symbol: String,
    market_data: MarketData,
    dydx_summary: Option<MarketSummary>,
    hl_summary: Option<MarketSummary>,
    dydx_leverage: Option<f64>,
    hl_leverage: Option<f64>,
    terminal: Arc<Mutex<Terminal<CrosstermBackend<Stdout>>>>,
    positions: Vec<Position>,
}

impl Drop for App {
    fn drop(&mut self) {
        // Cleanup terminal
        if let Ok(mut terminal) = self.terminal.try_lock() {
            let _ = terminal.show_cursor();
            let _ = terminal.clear();
            let _ = disable_raw_mode();
            let _ = execute!(
                terminal.backend_mut(),
                LeaveAlternateScreen,
            );
        }
    }
}

impl App {
    async fn new() -> Result<Self> {
        let config = AggregatorConfig::default();
        let aggregator = DerivativesAggregator::new(config).await?;
        let wallet_manager = WalletManager::new().await?;
        let hyperliquid_service = HyperliquidService::new(&wallet_manager).await?;
        
        // Initialize terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;
        
        Ok(Self {
            aggregator,
            hyperliquid_service,
            wallet_manager,
            selected_exchange: None,
            symbol: "BTC".to_string(),
            market_data: MarketData::default(),
            dydx_summary: None,
            hl_summary: None,
            dydx_leverage: None,
            hl_leverage: None,
            terminal: Arc::new(Mutex::new(terminal)),
            positions: Vec::new(),
        })
    }

    async fn update(&mut self) -> Result<()> {
        // Update all exchange data
        self.aggregator.start_all_market_updates(&self.symbol).await?;
        
        // Update summaries
        self.dydx_summary = self.aggregator
            .get_exchange_summary("dYdX", &self.symbol)
            .await
            .ok();
            
        self.hl_summary = self.aggregator
            .get_exchange_summary("Hyperliquid", &self.symbol)
            .await
            .ok();
        
        // Update leverage info
        self.dydx_leverage = match &self.aggregator.exchanges.get("dYdX") {
            Some(Exchange::Dydx(e)) => {
                e.get_leverage_info(&self.symbol).await.ok().map(|info| info.max_leverage)
            },
            _ => None
        };
            
        self.hl_leverage = match &self.aggregator.exchanges.get("Hyperliquid") {
            Some(Exchange::Hyperliquid(e)) => {
                e.get_leverage_info(&self.symbol).await.ok().map(|info| info.max_leverage)
            },
            _ => None
        };
        
        // Update selected exchange orderbook if one is selected
        if let Some(exchange) = &self.selected_exchange {
            if let Ok(orderbook) = self.aggregator.get_exchange_orderbook(exchange, &self.symbol).await {
                self.market_data.orderbook = Some(orderbook);
            }
        }
        
        // Update positions from both exchanges
        let mut all_positions = Vec::new();
        
        // Get Hyperliquid positions
        if let Ok(hl_positions) = self.hyperliquid_service.get_positions().await {
            all_positions.extend(hl_positions);
        }
        
        // Get dYdX positions
        if let Ok(dydx_positions) = self.wallet_manager.get_dydx_positions().await {
            all_positions.extend(dydx_positions);
        }

        // Update market data (we need to add positions field to MarketData)
        self.positions = all_positions;  // Store positions directly in App instead of MarketData
        
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    init_file_logging();
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new().await?;
    
    //std::env::set_var("RUST_LOG", "info");
    //env_logger::init();
    
    loop {
        // Update market data first
        if let Err(e) = app.update().await {
            eprintln!("Error updating market data: {}", e);
        }

        // Then draw UI using cached data
        terminal.draw(|f| ui(f, &app))?;
        
        // Handle input
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char(c) => {
                        if let Some(option) = MenuOption::from_str(&c.to_string()) {
                            match option {
                                MenuOption::ViewDydx => {
                                    app.selected_exchange = Some("dYdX".to_string());
                                    start_market_updates(&mut app.aggregator, &app.symbol).await?;
                                },
                                MenuOption::ViewHyperliquid => {
                                    app.selected_exchange = Some("Hyperliquid".to_string());
                                    start_market_updates(&mut app.aggregator, &app.symbol).await?;
                                },
                                MenuOption::ViewPositions => {
                                    let mut positions = Vec::new();
                                    
                                    // Get dYdX positions
                                    if let Ok(dydx_positions) = app.wallet_manager.get_dydx_positions().await {
                                        positions.extend(dydx_positions);
                                    }
                                    
                                    // Get Hyperliquid positions
                                    if let Ok(hl_positions) = app.hyperliquid_service.get_positions().await {
                                        positions.extend(hl_positions);
                                    }
                                    
                                    // Clear screen and show positions
                                    terminal.clear()?;
                                    terminal.draw(|f| {
                                        Position::display_positions(f, &positions);
                                    })?;
                                    
                                    // Wait for input to return to main menu
                                    if let Event::Key(key) = event::read()? {
                                        if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                                            continue;  // Return to main menu
                                        }
                                    }
                                },
                                MenuOption::ViewOpenOrders => {
                                    let mut orders = Vec::new();
                                    
                                    // Get dYdX orders
                                    if let Ok(dydx_orders) = app.wallet_manager.get_dydx_orders().await {
                                        // Filter for only open orders and convert to common Order type
                                        let open_orders: Vec<_> = dydx_orders.into_iter()
                                            .filter_map(|order| Order::from_dydx_order(&order).ok())
                                            .filter(|order| order.status == "Open")
                                            .collect();
                                        orders.extend(open_orders);
                                    }
                                    
                                    // Get Hyperliquid orders and convert to common Order type
                                    if let Ok(hl_orders) = app.hyperliquid_service.get_open_orders().await {
                                        let converted_orders: Vec<_> = hl_orders.into_iter()
                                            .filter_map(|order| Order::from_hl_order(&order).ok())
                                            .collect();
                                        orders.extend(converted_orders);
                                    }
                                    
                                    // Clear screen and display orders
                                    terminal.clear()?;
                                    terminal.draw(|f| {
                                        Order::display_orders(f, &orders);
                                    })?;
                                    
                                    // Wait for input to return to main menu
                                    if let Event::Key(key) = event::read()? {
                                        if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
                                            continue;
                                        }
                                    }
                                },
                                MenuOption::ChangeSymbol => {
                                    disable_raw_mode()?;
                                    terminal.clear()?;
                                    
                                    print!("\x1b[2J\x1b[1;1H"); // Clear screen
                                    println!("\x1b[1;36mChange Symbol\x1b[0m\n");
                                    print!("Enter new symbol: ");
                                    io::stdout().flush()?;
                                    
                                    let mut new_symbol = String::new();
                                    io::stdin().read_line(&mut new_symbol)?;
                                    
                                    app.symbol = new_symbol.trim().to_uppercase();
                                    app.aggregator.start_all_market_updates(&app.symbol).await?;
                                    
                                    enable_raw_mode()?;
                                    terminal.clear()?;
                                },
                                MenuOption::PlaceTrade => {
                                    if let Some(exchange) = app.selected_exchange.clone() {
                                        let symbol = app.symbol.clone();
                                        if let Err(e) = place_trade(&mut app, &symbol, &exchange).await {
                                            eprintln!("Error placing trade: {}", e);
                                        }
                                    }
                                },
                                MenuOption::ManageWallets => {
                                    manage_wallets(&mut app, &mut terminal).await?;
                                },
                                MenuOption::Exit => break,
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Cleanup
    disable_raw_mode()?;
    terminal.show_cursor()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    
    Ok(())
}

async fn start_market_updates(aggregator: &mut DerivativesAggregator, symbol: &str) -> Result<()> {
    aggregator.start_all_market_updates(symbol).await?;
    sleep(Duration::from_secs(2)).await; // Give time for initial data
    Ok(())
}

async fn place_trade(app: &mut App, symbol: &str, exchange: &str) -> Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    
    let mut log_message = None;
    
    loop {
        // Get latest orderbook
        let orderbook = app.aggregator.get_exchange_orderbook(exchange, symbol).await.ok();

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('1'..='4') => {
                        // Temporarily disable raw mode for input
                        disable_raw_mode()?;
                        
                        // Save current terminal state
                        terminal.clear()?;
                        terminal.draw(|f| {
                            trading_ui(f, symbol, exchange, orderbook.as_ref(), log_message.as_deref());
                        })?;
                        
                        // Move cursor to input position and get amount
                        execute!(
                            io::stdout(),
                            cursor::MoveTo(2, terminal.size()?.height - 2),
                            terminal::Clear(terminal::ClearType::CurrentLine)
                        )?;
                        print!("Enter USD value: $");
                        io::stdout().flush()?;
                        
                        let mut amount_input = String::new();
                        io::stdin().read_line(&mut amount_input)?;
                        let usd_value = amount_input.trim().parse()?;

                        let (order_type, is_buy) = match key.code {
                            KeyCode::Char('1') => (OrderType::Market, true),
                            KeyCode::Char('2') => (OrderType::Market, false),
                            KeyCode::Char('3') => (OrderType::Limit, true),
                            KeyCode::Char('4') => (OrderType::Limit, false),
                            _ => unreachable!(),
                        };

                        // Add leverage input
                        execute!(
                            io::stdout(),
                            cursor::MoveTo(2, terminal.size()?.height - 3),
                            terminal::Clear(terminal::ClearType::CurrentLine)
                        )?;
                        print!("Enter leverage: ");
                        io::stdout().flush()?;
                        
                        let mut leverage_input = String::new();
                        io::stdin().read_line(&mut leverage_input)?;
                        let leverage = leverage_input.trim().parse().unwrap_or(1);

                        // Add margin mode input
                        execute!(
                            io::stdout(),
                            cursor::MoveTo(2, terminal.size()?.height - 2),
                            terminal::Clear(terminal::ClearType::CurrentLine)
                        )?;
                        print!("Cross margin? (y/n): ");
                        io::stdout().flush()?;
                        
                        let mut margin_input = String::new();
                        io::stdin().read_line(&mut margin_input)?;
                        let cross_margin = margin_input.trim().to_lowercase().starts_with('y');

                        let mut price = None;
                        if matches!(order_type, OrderType::Limit) {
                            execute!(
                                io::stdout(),
                                cursor::MoveTo(2, terminal.size()?.height - 1),
                                terminal::Clear(terminal::ClearType::CurrentLine)
                            )?;
                            print!("Enter price: ");
                            io::stdout().flush()?;
                            
                            let mut price_input = String::new();
                            io::stdin().read_line(&mut price_input)?;
                            price = Some(price_input.trim().parse()?);
                        }

                        // Re-enable raw mode
                        enable_raw_mode()?;
                        terminal.clear()?;

                        let is_market = matches!(order_type, OrderType::Market);
                        let request = TradeRequest {
                            asset: symbol.to_string(),
                            order_type,
                            is_buy,
                            usd_value,
                            price: if is_market {
                                Some(0.0) // Use 0.0 for market orders
                            } else {
                                price
                            },
                            leverage,
                            reduce_only: false,
                            cross_margin,
                        };

                        // Route to correct exchange
                        let result = match exchange {
                            "dYdX" => {
                                // Convert local OrderType to dYdX OrderType
                                let dydx_order_type = match request.order_type {
                                    LocalOrderType::Market => DydxOrderType::Market,
                                    LocalOrderType::Limit => DydxOrderType::Limit,
                                };

                                app.wallet_manager.place_dydx_order(
                                    &symbol,
                                    if request.is_buy { OrderSide::Buy } else { OrderSide::Sell },
                                    request.usd_value,
                                    request.price,
                                    dydx_order_type,
                                    OrderTimeInForce::Ioc,
                                    leverage as f64,
                                ).await
                            },
                            "Hyperliquid" => {
                                app.hyperliquid_service.place_trade(request).await
                                    .map(|response| match response {
                                        ExchangeResponseStatus::Ok(response) => response.response_type,
                                        ExchangeResponseStatus::Err(message) => message,
                                        _ => "Unknown response status".to_string()
                                    })
                            },
                            _ => Err(anyhow::anyhow!("Unknown exchange: {}", exchange))
                        };

                        match result {
                            Ok(tx_hash) => {
                                log_message = Some(format!("Trade placed successfully: {}", tx_hash));
                            },
                            Err(e) => {
                                let error_msg = format!(
                                    "Error placing trade:\n\
                                     {}\n\
                                     Time: {}",
                                    e,
                                    chrono::Local::now().format("%H:%M:%S")
                                );
                                log_message = Some(error_msg);
                            }
                        }
                    },
                    KeyCode::Char('5') | KeyCode::Esc => {
                        terminal.clear()?;
                        return Ok(());
                    },
                    _ => {}
                }
            }

        }

        // Redraw the UI with log message
        terminal.draw(|f| {
            trading_ui(f, symbol, exchange, orderbook.as_ref(), log_message.as_deref());
        })?;
    }

    // Properly cleanup and restore main menu terminal state
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    
    Ok(())
}

fn ui(f: &mut ratatui::Frame<'_>, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),   // Menu
            Constraint::Length(10),  // Market Summaries
            Constraint::Min(0),      // Selected Exchange Data (Orderbook)
        ])
        .split(f.area());

    // Menu
    let menu = Paragraph::new("1. View Dydx  2. View Hyperliquid  3. View Positions  4. View Open Orders  5. Change Symbol  6. Place Trade  7. Manage Wallets  8. Exit")
        .block(Block::default().borders(Borders::ALL).title("Menu"));
    f.render_widget(menu, chunks[0]);

    // Market Summaries - Split horizontally for each exchanges
    let summary_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(50),  // dYdX
            Constraint::Percentage(50),  // Hyperliquid
        ])
        .split(chunks[1]);

    // dYdX Summary
    let dydx_summary = match &app.dydx_summary {
        Some(summary) => format!(
            "dYdX - {}\nPrice: ${}\n24h Volume: {}\nMax Leverage: {}\nFunding: {:.4}%",
            app.symbol,
            summary.price,
            format_volume(summary.volume_24h),
            app.dydx_leverage.map_or_else(|| "N/A".to_string(), |l| format!("{:.0}x", l)),
            summary.funding_rate * 100.0
        ),
        None => format!("dYdX - {}\nNo data available", app.symbol)
    };
    
    let dydx_widget = Paragraph::new(dydx_summary)
        .block(Block::default().borders(Borders::ALL).title("dYdX Market"));
    f.render_widget(dydx_widget, summary_chunks[0]);

    // Hyperliquid Summary
    let hl_summary = match &app.hl_summary {
        Some(summary) => format!(
            "Hyperliquid - {}\nPrice: ${}\n24h Volume: {}\nMax Leverage: {}\nFunding: {:.4}%",
            app.symbol,
            summary.price,
            format_volume(summary.volume_24h),
            app.hl_leverage.map_or_else(|| "N/A".to_string(), |l| format!("{:.0}x", l)),
            summary.funding_rate * 100.0
        ),
        None => format!("Hyperliquid - {}\nNo data available", app.symbol)
    };
    
    let hl_widget = Paragraph::new(hl_summary)
        .block(Block::default().borders(Borders::ALL).title("Hyperliquid Market"));
    f.render_widget(hl_widget, summary_chunks[1]);

    // Orderbook (if an exchange is selected)
    if let Some(orderbook) = &app.market_data.orderbook {
        let mut orderbook_text = String::new();
        
        // Helper function to format price with dynamic decimal places
        let format_price = |price: f64| -> String {
            if price < 10.0 {
                format!("${:.6}", price)
            } else {
                format!("${:.2}", price)
            }
        };
        
        // Display asks in red (reversed order)
        orderbook_text.push_str("\x1b[0mAsks:\n");
        orderbook_text.push_str("      Size          Price\n");
        orderbook_text.push_str("------------------------------\n");
        
        for ask in orderbook.asks.iter().rev().take(5) {
            orderbook_text.push_str(&format!("\x1b[31m{:>10.4}     {}\x1b[0m\n",
                ask.size,
                format_price(ask.price)
            ));
        }
        
        // Display bids in green
        orderbook_text.push_str("\x1b[0mBids:\n");
        for bid in orderbook.bids.iter().take(5) {
            orderbook_text.push_str(&format!("\x1b[32m{:>10.4}     {}\x1b[0m\n",
                bid.size,
                format_price(bid.price)
            ));
        }
        
        let orderbook_title = format!("{} Orderbook", orderbook.exchange);
        let orderbook_widget = Paragraph::new(orderbook_text)
            .block(Block::default().borders(Borders::ALL).title(orderbook_title));
        f.render_widget(orderbook_widget, chunks[2]);
    }
}

fn format_volume(volume: f64) -> String {
    if volume >= 1_000_000_000.0 {
        format!("${:.2}B", volume / 1_000_000_000.0)
    } else if volume >= 1_000_000.0 {
        format!("${:.2}M", volume / 1_000_000.0)
    } else if volume >= 1_000.0 {
        format!("${:.2}K", volume / 1_000.0)
    } else {
        format!("${:.2}", volume)
    }
}

fn trading_ui(f: &mut ratatui::Frame<'_>, symbol: &str, exchange: &str, orderbook: Option<&OrderBook>, log_message: Option<&str>) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(70),  // Main content - reduced from Min(0)
            Constraint::Percentage(30),  // Log area - increased from Length(3)
        ])
        .split(f.area());

    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),  // Trading menu
            Constraint::Percentage(70),  // Orderbook
        ])
        .split(chunks[0]);

    // Trading Menu (existing code)
    let menu_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),   // Title
            Constraint::Length(8),   // Trading options
            Constraint::Min(0),      // Remaining space
        ])
        .split(main_chunks[0]);

    // Title
    let title = Paragraph::new(format!("Trading {} on {}", symbol, exchange))
        .block(Block::default().borders(Borders::ALL))
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(title, menu_chunks[0]);

    // Trading Options
    let options = Paragraph::new(
        "1. Market Buy\n2. Market Sell\n3. Limit Buy\n4. Limit Sell\n5. Back to Main Menu"
    )
    .block(Block::default().borders(Borders::ALL).title("Options"));
    f.render_widget(options, menu_chunks[1]);

    // Orderbook (reuse existing orderbook display code)
    if let Some(orderbook) = orderbook {
        let mut orderbook_text = String::new();
        
        // Helper function to format price with dynamic decimal places
        let format_price = |price: f64| -> String {
            if price < 10.0 {
                format!("${:>10.6}", price)
            } else {
                format!("${:>10.2}", price)
            }
        };
        
        // Display asks in red (reversed order)
        orderbook_text.push_str("\x1b[0mAsks:\n");
        orderbook_text.push_str("      Size          Price\n");
        orderbook_text.push_str("------------------------------\n");
        
        for ask in orderbook.asks.iter().rev().take(5) {
            orderbook_text.push_str(&format!("\x1b[31m{:>10.4}     {}\x1b[0m\n",
                ask.size,
                format_price(ask.price)
            ));
        }
        
        // Show market price
        if let (Some(lowest_ask), Some(highest_bid)) = (orderbook.asks.first(), orderbook.bids.first()) {
            let market_price = (lowest_ask.price + highest_bid.price) / 2.0;
            orderbook_text.push_str("\x1b[0m------------------------------\n");
            orderbook_text.push_str(&format!("Market Price: ${:.2}\n", market_price));
            orderbook_text.push_str("------------------------------\n");
        }
        
        // Display bids in green
        orderbook_text.push_str("\x1b[0mBids:\n");
        for bid in orderbook.bids.iter().take(5) {
            orderbook_text.push_str(&format!("\x1b[32m{:>10.4}     {}\x1b[0m\n",
                bid.size,
                format_price(bid.price)
            ));
        }
        
        let orderbook_title = format!("{} Orderbook", orderbook.exchange);
        let orderbook_widget = Paragraph::new(orderbook_text)
            .block(Block::default().borders(Borders::ALL).title(orderbook_title));
        f.render_widget(orderbook_widget, main_chunks[1]);
    }

    // Add log area with increased size
    if let Some(message) = log_message {
        let log = Paragraph::new(message)
            .block(Block::default()
                .borders(Borders::ALL)
                .title("Trade Log"))
            .wrap(ratatui::widgets::Wrap { trim: true });  // Add text wrapping
        f.render_widget(log, chunks[1]);
    }
}

fn display_open_orders(terminal: &mut Terminal<CrosstermBackend<Stdout>>, orders: &[OpenOrder]) -> Result<()> {
    terminal.draw(|f| {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),    // Title
                Constraint::Min(0),       // Orders
                Constraint::Length(3),    // Menu
            ].as_ref())
            .split(f.size());

        // Title
        let title = Paragraph::new("Open Orders")
            .block(Block::default().borders(Borders::ALL))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Orders content
        let mut orders_text = String::new();
        orders_text.push_str("Asset    Side    USD Amount    Price    Order ID\n");
        orders_text.push_str("------------------------------------------------\n");

        for order in orders {
            let usd_amount = order.size * order.price;
            orders_text.push_str(&format!(
                "{:<8} {:<7} ${:<11.2} {:<8.2} {}\n",
                order.asset,
                order.side,
                usd_amount,
                order.price,
                order.order_id,
            ));
        }

        if orders.is_empty() {
            orders_text.push_str("\nNo open orders");
        }

        let orders_widget = Paragraph::new(orders_text)
            .block(Block::default()
                .borders(Borders::ALL)
                .title("Orders List"));

        f.render_widget(orders_widget, chunks[1]);

        // Menu
        let menu = Paragraph::new("Press 'q' to return to main menu")
            .block(Block::default().borders(Borders::ALL))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(menu, chunks[2]);
    })?;

    // Wait for key press to return to menu
    loop {
        if let Event::Key(key) = event::read()? {
            if let KeyCode::Char('q') | KeyCode::Esc = key.code {
                break;
            }
        }
    }

    Ok(())
}

async fn manage_wallets(app: &mut App, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    loop {
        // Initialize dYdX client before getting balance
        app.wallet_manager.init_dydx_client().await?;
        
        // Get wallet info and dYdX balance
        let wallet_info = app.wallet_manager.get_wallet_info().await?;
        let dydx_balance = app.wallet_manager.get_dydx_balance().await?;
        
        // Draw UI
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(3),     // Title
                    Constraint::Length(12),    // Wallet Status (increased from 8)
                    Constraint::Length(8),     // Options Menu
                    Constraint::Length(3),     // Input Prompt
                ].as_ref())
                .split(f.size());

            // Title
            let title = Paragraph::new("Wallet Management")
                .block(Block::default().borders(Borders::ALL))
                .alignment(ratatui::layout::Alignment::Center);
            f.render_widget(title, chunks[0]);

            // Wallet Status
            let mut status_text = String::new();
            if let Some(wallet) = app.wallet_manager.get_wallet() {
                status_text.push_str(&format!("ETH Address: {:#x}\n", wallet.address()));
                status_text.push_str(&format!("USDC Balance: ${:.2}\n", wallet_info.4));
                status_text.push_str(&format!("Hyperliquid Portifolio Value: ${:.2}\n", wallet_info.2));
                status_text.push_str(&format!("Hyperliquid Margin Used: ${:.2}\n", wallet_info.3));
                let hl_balance = wallet_info.2 - wallet_info.3;
                status_text.push_str(&format!("Hyperliquid Balance: ${:.2}\n", hl_balance));
            } else {
                status_text.push_str("No ETH wallet configured\n");
            }

            if let Some(dydx_wallet) = app.wallet_manager.get_dydx_wallet() {
                if let Ok(account) = dydx_wallet.account_offline(0) {
                    status_text.push_str(&format!("dYdX Address: {}\n", account.address()));
                    if let Some(balance) = dydx_balance {
                        status_text.push_str(&format!("dYdX Balance: ${:.2}\n", balance));
                    }
                }
            } else {
                status_text.push_str("No dYdX wallet configured\n");
            }

            let status = Paragraph::new(status_text)
                .block(Block::default().borders(Borders::ALL).title("Wallet Status"));
            f.render_widget(status, chunks[1]);

            // Options Menu
            let options = Paragraph::new(
                "1. Create New ETH Wallet\n\
                 2. Import Existing ETH Wallet\n\
                 3. Create New dYdX Wallet\n\
                 4. Import Existing dYdX Wallet\n\
                 5. Bridge USDC to dYdX\n\
                 6. Back to Main Menu"
            )
            .block(Block::default().borders(Borders::ALL).title("Options"));
            f.render_widget(options, chunks[2]);

            // Input Prompt
            let prompt = Paragraph::new("Enter choice (1-6): ")
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(prompt, chunks[3]);
        })?;

        // Handle input
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('1') => app.wallet_manager.create_eth_wallet().await?,
                KeyCode::Char('2') => app.wallet_manager.import_eth_wallet().await?,
                KeyCode::Char('3') => app.wallet_manager.create_dydx_wallet().await?,
                KeyCode::Char('4') => app.wallet_manager.import_dydx_wallet().await?,
                KeyCode::Char('5') => {
                    // Bridge USDC
                    terminal.clear()?;
                    disable_raw_mode()?;
                    
                    print!("Enter USDC amount to bridge: ");
                    io::stdout().flush()?;
                    
                    let mut input = String::new();
                    io::stdin().read_line(&mut input)?;
                    let amount = input.trim().parse::<f64>()?;
                    
                    println!("Initiating bridge of {} USDC to dYdX...", amount);
                    app.wallet_manager.bridge_to_dydx(amount).await?;
                    
                    println!("\nPress Enter to continue...");
                    io::stdin().read_line(&mut input)?;
                    
                    enable_raw_mode()?;
                    terminal.clear()?;
                },
                KeyCode::Char('6') | KeyCode::Char('q') | KeyCode::Esc => {
                    // Clear screen before exiting
                    terminal.clear()?;
                    break;
                },
                _ => {}
            }
        }
    }

    // Ensure terminal is properly reset before returning to main menu
    terminal.clear()?;
    terminal.draw(|f| {
        // Draw empty frame to ensure clean state
        let block = Block::default();
        f.render_widget(block, f.size());
    })?;

    Ok(())
}