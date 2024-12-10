use dydx::indexer::types::{OrderResponseObject, OrderSide, OrderStatus, ApiOrderStatus, OrderFlags};
use crate::trading::hyperliquid_service::OpenOrder;
use anyhow::Result;
use num_traits::ToPrimitive;
use ratatui::{
    widgets::{Block, Borders, Paragraph},
    layout::{Layout, Constraint, Direction},
};

#[derive(Debug, Clone)]
pub struct Order {
    pub exchange: String,
    pub asset: String,
    pub size: f64,
    pub price: f64,
    pub side: String,
    pub status: String,
    pub order_id: String,
}

impl Order {
    pub fn from_dydx_order(order: &OrderResponseObject) -> Result<Self> {
        Ok(Order {
            exchange: "dYdX".to_string(),
            asset: order.ticker.0.clone(),
            size: order.size.0.to_f64().unwrap_or(0.0),
            price: order.price.0.to_f64().unwrap_or(0.0),
            side: match order.side {
                OrderSide::Buy => "Buy".to_string(),
                OrderSide::Sell => "Sell".to_string(),
            },
            status: match &order.status {
                ApiOrderStatus::OrderStatus(status) => match status {
                    OrderStatus::Open => "Open".to_string(),
                    OrderStatus::Filled => "Filled".to_string(),
                    OrderStatus::Canceled => "Canceled".to_string(),
                    OrderStatus::BestEffortCanceled => "BestEffortCanceled".to_string(),
                    OrderStatus::Untriggered => "Untriggered".to_string(),
                },
                ApiOrderStatus::BestEffort(_) => "BestEffort".to_string(),
            },
            order_id: format!("{}:{}:{}:{}",
                order.client_id.0,
                order.clob_pair_id.0,
                match order.order_flags {
                    OrderFlags::ShortTerm => 0,
                    OrderFlags::Conditional => 32,
                    OrderFlags::LongTerm => 64,
                },
                order.subaccount_id.0
            ),
        })
    }

    pub fn from_hl_order(order: &OpenOrder) -> Result<Self> {
        Ok(Order {
            exchange: "Hyperliquid".to_string(),
            asset: order.asset.clone(),
            size: order.size,
            price: order.price,
            side: match order.side.as_str() {
                "b" | "B" | "buy" | "Buy" => "Buy".to_string(),
                _ => "Sell".to_string(),
            },
            status: "Open".to_string(),
            order_id: order.order_id.to_string(),
        })
    }

    pub fn display_orders(f: &mut ratatui::Frame, orders: &[Order]) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),    // Title
                Constraint::Min(0),       // Orders
                Constraint::Length(3),    // Menu
            ])
            .split(f.area());

        // Update the title to show exchange breakdown
        let dydx_count = orders.iter().filter(|p| p.exchange == "dYdX").count();
        let hl_count = orders.iter().filter(|p| p.exchange == "Hyperliquid").count();
        let title = format!("Open Orders (dYdX: {}, Hyperliquid: {})", dydx_count, hl_count);
        
        let title_widget = Paragraph::new(title)
            .block(Block::default().borders(Borders::ALL))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(title_widget, chunks[0]);

        // Calculate height for each order
        let order_height = 4;
        let order_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                orders.iter()
                    .map(|_| Constraint::Length(order_height))
                    .collect::<Vec<_>>()
            )
            .split(chunks[1]);

        // Render orders
        for (idx, order) in orders.iter().enumerate() {
            let usd_value = order.size * order.price;
            let order_text = format!(
                "#{}: Size: {} {} | Value: ${:.2}\nPrice: ${:.2}\nSide: {}\nStatus: {}",
                idx + 1,
                order.size,
                order.asset,
                usd_value,
                order.price,
                order.side,
                order.status
            );
            
            let order_widget = Paragraph::new(order_text)
                .block(Block::default()
                    .borders(Borders::ALL)
                    .title(format!("{} Order ({})", order.asset, order.exchange)));
            f.render_widget(order_widget, order_chunks[idx]);
        }

        // Menu
        let menu = Paragraph::new("Press 'q' to return to main menu, type id to cancel/close")
            .block(Block::default().borders(Borders::ALL))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(menu, chunks[2]);
    }
}