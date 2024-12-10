use hyperliquid_rust_sdk::PositionData;
use anyhow::Result;
use ratatui::prelude::*;
use ratatui::{
    widgets::{Block, Borders, Paragraph},
    layout::{Layout, Constraint, Direction},
};
use dydx::indexer::PerpetualPositionResponseObject;
use num_traits::ToPrimitive;
use dydx::indexer::types::PositionSide;

#[derive(Debug, Clone)]
pub struct Position {
    pub exchange: String,
    pub asset: String,
    pub size: f64,
    pub entry_price: Option<f64>,
    pub liquidation_price: Option<f64>,
    pub unrealized_pnl: f64,
    pub margin_used: Option<f64>,
    pub leverage: Option<u32>,
    pub roe: Option<f64>,
    pub side: String,
}

impl Position {
    pub fn from_position_data(data: &PositionData) -> Result<Self> {
        Ok(Position {
            exchange: "Hyperliquid".to_string(),
            asset: data.coin.clone(),
            size: data.szi.parse::<f64>()?,
            entry_price: data.entry_px.as_ref().and_then(|p| p.parse().ok()),
            liquidation_price: data.liquidation_px.as_ref().and_then(|p| p.parse().ok()),
            unrealized_pnl: data.unrealized_pnl.parse::<f64>()?,
            margin_used: Some(data.margin_used.parse::<f64>()?),
            leverage: Some(data.leverage.value as u32),
            roe: Some(data.return_on_equity.parse::<f64>()?),
            side: "".to_string(),
        })
    }

    pub fn from_dydx_position(pos: &PerpetualPositionResponseObject) -> Result<Self> {
        Ok(Position {
            exchange: "dYdX".to_string(),
            asset: pos.market.0.clone(),
            size: pos.size.0.to_f64().unwrap_or(0.0),
            entry_price: Some(pos.entry_price.0.to_f64().unwrap_or(0.0)),
            liquidation_price: None,
            unrealized_pnl: pos.unrealized_pnl.to_f64().unwrap_or(0.0),
            margin_used: None,
            leverage: None,
            roe: None,
            side: match pos.side {
                PositionSide::Long => "Long".to_string(),
                PositionSide::Short => "Short".to_string(),
                _ => "Unknown".to_string(),
            },
        })
    }

    fn format_position(&self) -> String {
        let mut lines = vec![
            format!("Size: {} {}", self.size, self.side),
            format!("Entry Price: ${:.2}", self.entry_price.unwrap_or(0.0)),
        ];

        if let Some(liq_price) = self.liquidation_price {
            lines.push(format!("Liquidation Price: ${:.2}", liq_price));
        }

        lines.push(format!("Unrealized PnL: ${:.2}", self.unrealized_pnl));

        if let Some(margin) = self.margin_used {
            lines.push(format!("Margin Used: ${:.2}", margin));
        }

        if let Some(lev) = self.leverage {
            lines.push(format!("Leverage: {}x", lev));
        }

        if let Some(roe) = self.roe {
            lines.push(format!("ROE: {:.2}%", roe * 100.0));
        }

        lines.join("\n")
    }

    pub fn display_positions(f: &mut Frame<'_>, positions: &[Position]) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),    // Title
                Constraint::Min(0),       // Positions
                Constraint::Length(3),    // Menu
            ])
            .split(f.area());

        // Update the title to show exchange breakdown
        let dydx_count = positions.iter().filter(|p| p.exchange == "dYdX").count();
        let hl_count = positions.iter().filter(|p| p.exchange == "Hyperliquid").count();
        let title = format!("Current Positions (dYdX: {}, Hyperliquid: {})", dydx_count, hl_count);
        
        let title_widget = Paragraph::new(title)
            .block(Block::default().borders(Borders::ALL))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(title_widget, chunks[0]);

        // Calculate height for each position based on available fields
        let position_heights: Vec<u16> = positions.iter()
            .map(|p| {
                let mut height = 6; // Base height for common fields
                if p.margin_used.is_some() { height += 1; }
                if p.leverage.is_some() { height += 1; }
                if p.roe.is_some() { height += 1; }
                height
            })
            .collect();

        // Create position chunks with variable heights
        let position_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                position_heights.iter()
                    .map(|&h| Constraint::Length(h))
                    .collect::<Vec<_>>()
            )
            .split(chunks[1]);

        // Render positions
        for (idx, position) in positions.iter().enumerate() {
            let position_text = Self::format_position(position);
            let position_widget = Paragraph::new(position_text)
                .block(Block::default()
                    .borders(Borders::ALL)
                    .title(format!("{} Position ({})", position.asset, position.exchange)));
            f.render_widget(position_widget, position_chunks[idx]);
        }

        // Menu
        let menu = Paragraph::new("Press 'q' to return to main menu")
            .block(Block::default().borders(Borders::ALL))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(menu, chunks[2]);
    }
}