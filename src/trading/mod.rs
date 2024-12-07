use serde::{Deserialize, Serialize};

pub mod hyperliquid_service;
pub mod positions;
pub mod wallet;

#[derive(Debug, Serialize, Deserialize)]
pub enum OrderType {
    Market,
    Limit,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TradeRequest {
    pub asset: String,
    pub order_type: OrderType,
    pub is_buy: bool,
    pub amount: f64,
    pub price: Option<f64>,
    pub leverage: u32,
    pub reduce_only: bool,
}