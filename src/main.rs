use hl_aggregator::{
    AggregatorConfig,
    aggregator::{
        Exchange,
        DerivativesAggregator,
    },
};
use anyhow::Result;
use tokio::time::{sleep, Duration};
use std::io::{self, Write, stdin};
use hl_aggregator::trading::{OrderType, TradeRequest};
use hl_aggregator::trading::hyperliquid_service::HyperliquidService;
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
};
use hl_aggregator::aggregator::types::{MarketData, MarketSummary};

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

struct App {
    aggregator: DerivativesAggregator,
    selected_exchange: Option<String>,
    symbol: String,
    market_data: MarketData,
    dydx_summary: Option<MarketSummary>,
    hl_summary: Option<MarketSummary>,
    dydx_leverage: Option<f64>,
    hl_leverage: Option<f64>,
}

impl App {
    async fn new() -> Result<Self> {
        let config = AggregatorConfig::default();
        let aggregator = DerivativesAggregator::new(config).await?;
        
        Ok(Self {
            aggregator,
            selected_exchange: None,
            symbol: "BTC".to_string(),
            market_data: MarketData::default(),
            dydx_summary: None,
            hl_summary: None,
            dydx_leverage: None,
            hl_leverage: None,
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
        
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new().await?;
    
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
                                    if let Some(exchange) = &app.selected_exchange {
                                        let wallet_manager = WalletManager::new()?;
                                        let trading_service = HyperliquidService::new(&wallet_manager).await?;
                                        let positions = trading_service.get_positions().await?;
                                        
                                        println!("\x1b[2J\x1b[1;1H"); // Clear screen
                                        println!("\x1b[1;36mCurrent Positions:\x1b[0m\n");
                                        
                                        for position in positions {
                                            position.display();
                                        }
                                        
                                        println!("\nPress any key to continue...");
                                        let _ = blocking_read_line()?;
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
                                    if let Some(exchange) = &app.selected_exchange {
                                        if let Err(e) = handle_trade(exchange, &app.symbol).await {
                                            eprintln!("Trade error: {}", e);
                                        }
                                    }
                                },
                                MenuOption::ManageWallets => {
                                    let mut wallet_manager = WalletManager::new()?;
                                    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
                                    terminal.clear()?;
                                    
                                    let mut wallet_info = WalletInfo::default();
                                    
                                    // Update initial wallet info
                                    if let Ok((address, account_value, margin_used)) = wallet_manager.get_wallet_info().await {
                                        wallet_info.address = address;
                                        wallet_info.account_value = account_value;
                                        wallet_info.margin_used = margin_used;
                                    }
                                    
                                    loop {
                                        terminal.draw(|f| {
                                            wallet_management_ui(f, &wallet_info)
                                        })?;
                                        
                                        if event::poll(Duration::from_millis(100))? {
                                            if let Event::Key(key) = event::read()? {
                                                match key.code {
                                                    KeyCode::Char('1') => {
                                                        if confirm_action(&mut terminal, "Creating a new wallet will replace the existing one. Continue?").await? {
                                                            if let Ok(()) = wallet_manager.create_new_wallet().await {
                                                                if let Ok((address, account_value, margin_used)) = wallet_manager.get_wallet_info().await {
                                                                    wallet_info.address = address;
                                                                    wallet_info.account_value = account_value;
                                                                    wallet_info.margin_used = margin_used;
                                                                    wallet_info.log_messages.push("New wallet created successfully.".to_string());
                                                                }
                                                            } else {
                                                                wallet_info.log_messages.push("Failed to create new wallet.".to_string());
                                                            }
                                                        }
                                                    },
                                                    KeyCode::Char('2') => {
                                                        if confirm_action(&mut terminal, "Importing a wallet will replace the existing one. Continue?").await? {
                                                            if let Ok(()) = wallet_manager.import_wallet().await {
                                                                if let Ok((address, account_value, margin_used)) = wallet_manager.get_wallet_info().await {
                                                                    wallet_info.address = address;
                                                                    wallet_info.account_value = account_value;
                                                                    wallet_info.margin_used = margin_used;
                                                                    wallet_info.log_messages.push("Wallet imported successfully.".to_string());
                                                                }
                                                            } else {
                                                                wallet_info.log_messages.push("Failed to import wallet.".to_string());
                                                            }
                                                        }
                                                    },
                                                    KeyCode::Char('3') | KeyCode::Esc => {
                                                        terminal.clear()?;
                                                        break;
                                                    },
                                                    _ => {}
                                                }
                                            }
                                        }
                                    }
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

fn non_blocking_read_line() -> io::Result<String> {
    let mut input = String::new();
    stdin().read_line(&mut input)?;
    Ok(input)
}

fn blocking_read_line() -> io::Result<String> {
    let mut input = String::new();
    stdin().read_line(&mut input)?;
    Ok(input)
}

async fn handle_trade(exchange: &str, symbol: &str) -> Result<()> {
    print!("Enter amount: ");
    io::stdout().flush().map_err(|e| anyhow::anyhow!("{}", e))?;
    
    let mut amount = String::new();
    io::stdin()
        .read_line(&mut amount)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    
    let amount = amount.trim().parse::<f64>()
        .map_err(|e| anyhow::anyhow!("Invalid amount: {}", e))?;

    print!("Market (m) or Limit (l) order? ");
    io::stdout().flush().map_err(|e| anyhow::anyhow!("{}", e))?;
    
    let mut order_type = String::new();
    io::stdin()
        .read_line(&mut order_type)
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let _order_type = match order_type.trim().to_lowercase().as_str() {
        "m" => OrderType::Market,
        "l" => {
            print!("Enter price: ");
            io::stdout().flush().map_err(|e| anyhow::anyhow!("{}", e))?;
            
            let mut price_input = String::new();
            io::stdin()
                .read_line(&mut price_input)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            
            let _price = price_input.trim().parse::<f64>()
                .map_err(|e| anyhow::anyhow!("Invalid price: {}", e))?;
            
            OrderType::Limit
        }
        _ => return Err(anyhow::anyhow!("Invalid order type")),
    };

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

                let request = TradeRequest {
                    asset: symbol.to_string(),
                    order_type,
                    is_buy,
                    amount,
                    price: None,
                    leverage: 1,
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
    let menu = Paragraph::new("1. View Dydx  2. View Hyperliquid  3. View Positions  4. Change Symbol  5. Place Trade  6. Manage Wallets  7. Exit")
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
            "dYdX - {}\nPrice: ${:.2}\n24h Volume: ${:.2}M\nMax Leverage: {}\nFunding: {:.4}%",
            app.symbol,
            summary.price,
            summary.volume_24h / 1_000_000.0,
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
            "Hyperliquid - {}\nPrice: ${:.2}\n24h Volume: ${:.2}M\nMax Leverage: {}\nFunding: {:.4}%",
            app.symbol,
            summary.price,
            summary.volume_24h / 1_000_000.0,
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
        
        // Show spread
        if let (Some(lowest_ask), Some(highest_bid)) = (orderbook.asks.first(), orderbook.bids.first()) {
            let spread = lowest_ask.price - highest_bid.price;
            orderbook_text.push_str("\x1b[0m------------------------------\n");
            orderbook_text.push_str(&format!("Spread: {}\n", format_price(spread)));
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

fn wallet_management_ui(f: &mut ratatui::Frame<'_>, wallet_info: &WalletInfo) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),    // Title
            Constraint::Length(8),    // Wallet Info
            Constraint::Length(2),    // Spacing
            Constraint::Length(7),    // Menu Options
            Constraint::Length(10),   // Log Area
            Constraint::Min(0),       // Remaining space
        ])
        .split(f.area());

    // Title
    let title = Paragraph::new("Wallet Management")
        .block(Block::default().borders(Borders::ALL))
        .alignment(ratatui::layout::Alignment::Center);
    f.render_widget(title, chunks[0]);

    // Wallet Info
    let wallet_status = match &wallet_info.address {
        Some(addr) => format!(
            "Current Wallet: {}\nAccount Value: ${:.2} USD\nMargin Used: ${:.2} USD",
            addr, 
            wallet_info.account_value,
            wallet_info.margin_used
        ),
        None => "No wallet configured".to_string()
    };

    let wallet_widget = Paragraph::new(wallet_status)
        .block(Block::default().borders(Borders::ALL).title("Wallet Status"));
    f.render_widget(wallet_widget, chunks[1]);

    // Menu
    let menu = Paragraph::new(
        "1. Create New Wallet\n2. Import Existing Wallet\n3. Back to Main Menu"
    )
    .block(Block::default().borders(Borders::ALL).title("Options"));
    f.render_widget(menu, chunks[3]);

    // Log Area
    let log_content = wallet_info.log_messages.join("\n");
    let log_widget = Paragraph::new(log_content)
        .block(Block::default().borders(Borders::ALL).title("Log Messages"));
    f.render_widget(log_widget, chunks[4]);
}

async fn confirm_action(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, message: &str) -> Result<bool> {
    terminal.clear()?;
    
    // Create a Rect from the terminal size
    let size = terminal.size()?;
    let area = Rect::new(0, 0, size.width, size.height);
    
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

    terminal.draw(|f| {
        let confirm = Paragraph::new(format!("{} (y/n)", message))
            .block(Block::default().borders(Borders::ALL))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(confirm, layout[0]);
    })?;

    loop {
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => return Ok(true),
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => return Ok(false),
                    _ => {}
                }
            }
        }
    }
}

#[derive(Default)]
struct WalletInfo {
    address: Option<String>,
    account_value: f64,
    margin_used: f64,
    log_messages: Vec<String>,
}

