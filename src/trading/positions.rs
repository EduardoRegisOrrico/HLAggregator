use hyperliquid_rust_sdk::PositionData;
use anyhow::Result;

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

    pub fn display(&self) {
        println!("\n{:-^50}", format!(" {} Position ", self.asset));
        println!("Size: {:.4}", self.size);
        if let Some(entry) = self.entry_price {
            println!("Entry Price: ${:.2}", entry);
        }
        if let Some(liq) = self.liquidation_price {
            println!("Liquidation Price: ${:.2}", liq);
        }
        println!("Unrealized PnL: ${:.2}", self.unrealized_pnl);
        println!("Margin Used: ${:.2}", self.margin_used);
        println!("Leverage: {}x", self.leverage);
        println!("ROE: {:.2}%", self.roe * 100.0);
        println!("{:-^50}\n", "");
    }
}