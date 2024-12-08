use hyperliquid_rust_sdk::PositionData;
use anyhow::Result;
use crossterm::terminal::enable_raw_mode;
use crossterm::event::{self, Event, KeyCode};
use crossterm::terminal::disable_raw_mode;
use ratatui::prelude::*;
use std::io;
use std::time::Duration;
use ratatui::{
    backend::CrosstermBackend,
    widgets::{Block, Borders, Paragraph},
    layout::{Layout, Constraint, Direction},
    Terminal,
};

#[derive(Debug, Clone)]
pub struct Position {
    pub asset: String,
    pub size: f64,
    pub entry_price: Option<f64>,
    pub liquidation_price: Option<f64>,
    pub unrealized_pnl: f64,
    pub margin_used: f64,
    pub leverage: u32,
    pub roe: f64,
}

impl Position {
    pub fn from_position_data(data: &PositionData) -> Result<Self> {
        Ok(Position {
            asset: data.coin.clone(),
            size: data.szi.parse::<f64>()?,
            entry_price: data.entry_px.as_ref().and_then(|p| p.parse().ok()),
            liquidation_price: data.liquidation_px.as_ref().and_then(|p| p.parse().ok()),
            unrealized_pnl: data.unrealized_pnl.parse::<f64>()?,
            margin_used: data.margin_used.parse::<f64>()?,
            leverage: data.leverage.value as u32,
            roe: data.return_on_equity.parse::<f64>()?,
        })
    }

    fn format_position(position: &Position) -> String {
        let mut output = String::new();
        
        // Position details with consistent spacing
        output.push_str(&format!("Size: {:>38}\n", format!("{:+.4}", position.size)));
        
        if let Some(entry) = position.entry_price {
            output.push_str(&format!("Entry Price: {:>32}\n", format!("${:.2}", entry)));
        }
        
        if let Some(liq) = position.liquidation_price {
            output.push_str(&format!("Liquidation: {:>32}\n", format!("${:.2}", liq)));
        }
        
        let pnl_color = if position.unrealized_pnl >= 0.0 { "\x1b[32m" } else { "\x1b[31m" };
        output.push_str(&format!("Unrealized PnL: {:>30}\n", 
            format!("{}${:.2}\x1b[0m", pnl_color, position.unrealized_pnl)));
        
        output.push_str(&format!("Margin Used: {:>34}\n", format!("${:.2}", position.margin_used)));
        output.push_str(&format!("Leverage: {:>37}\n", format!("{}x", position.leverage)));
        
        let roe_color = if position.roe >= 0.0 { "\x1b[32m" } else { "\x1b[31m" };
        output.push_str(&format!("ROE: {:>41}\n", 
            format!("{}{:.2}%\x1b[0m", roe_color, position.roe * 100.0)));
        
        output
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

        // Title
        let title = Paragraph::new("Current Positions")
            .block(Block::default().borders(Borders::ALL))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(title, chunks[0]);

        // Create a layout for positions
        let position_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                positions.iter()
                    .map(|_| Constraint::Length(10))  // Height for each position
                    .collect::<Vec<_>>()
            )
            .split(chunks[1]);

        // Render each position in its own chunk
        for (idx, position) in positions.iter().enumerate() {
            let position_text = Self::format_position(position);
            let position_widget = Paragraph::new(position_text)
                .block(Block::default()
                    .borders(Borders::ALL)
                    .title(format!("{} Position", position.asset)));
            f.render_widget(position_widget, position_chunks[idx]);
        }

        // Menu
        let menu = Paragraph::new("Press 'q' to return to main menu")
            .block(Block::default().borders(Borders::ALL))
            .alignment(ratatui::layout::Alignment::Center);
        f.render_widget(menu, chunks[2]);
    }
}