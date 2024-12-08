use ethers::signers::LocalWallet;
use hyperliquid_rust_sdk::{
    BaseUrl, ClientLimit, ClientOrder, ClientOrderRequest, ExchangeClient,
    ExchangeResponseStatus, InfoClient,
};
use std::env;
use anyhow::Result;
use super::{OrderType, TradeRequest};
use super::positions::Position;
use ethers::signers::Signer;
use super::wallet::WalletManager;
use log;
use reqwest::Client;
use serde_json::json;

pub struct HyperliquidService {
    info_client: InfoClient,
    exchange_client: ExchangeClient,
}

impl HyperliquidService {
    pub async fn new(wallet_manager: &WalletManager) -> Result<Self> {
        let wallet = wallet_manager.get_wallet()
            .ok_or_else(|| anyhow::anyhow!("No wallet configured"))?;

        let exchange_client = ExchangeClient::new(
            None,
            wallet.clone(),
            Some(BaseUrl::Mainnet),
            None,
            None
        ).await?;

        let info_client = InfoClient::new(None, Some(BaseUrl::Mainnet)).await?;

        Ok(Self {
            info_client,
            exchange_client,
        })
    }

    pub async fn place_trade(&self, request: TradeRequest) -> Result<ExchangeResponseStatus> {
        // Get current orderbook and metadata
        let meta = self.info_client.meta().await?;
        let orderbook = self.info_client.l2_snapshot(request.asset.clone()).await?;
        
        // Get best bid/ask prices from the orderbook
        let (best_bid, best_ask) = {
            let best_bid = orderbook.levels.get(0)
                .and_then(|levels| levels.first())
                .map(|level| level.px.parse::<f64>())
                .transpose()
                .map_err(|_| anyhow::anyhow!("Failed to parse bid price"))?
                .ok_or_else(|| anyhow::anyhow!("No bid price available"))?;

            let best_ask = orderbook.levels.get(1)
                .and_then(|levels| levels.first())
                .map(|level| level.px.parse::<f64>())
                .transpose()
                .map_err(|_| anyhow::anyhow!("Failed to parse ask price"))?
                .ok_or_else(|| anyhow::anyhow!("No ask price available"))?;

            (best_bid, best_ask)
        };

        // Get current price based on order side
        let current_price = if request.is_buy { best_ask } else { best_bid };

        // Get asset metadata to determine size decimals
        let asset_meta = meta.universe
            .iter()
            .find(|asset| asset.name == request.asset)
            .ok_or_else(|| anyhow::anyhow!("Asset metadata not found"))?;

        // Calculate size from USD value and round to appropriate decimals
        let size = request.usd_value / current_price;
        let size = (size * 10_f64.powi(asset_meta.sz_decimals as i32)).round() / 10_f64.powi(asset_meta.sz_decimals as i32);

        // Ensure size is not zero after rounding
        if size == 0.0 {
            return Err(anyhow::anyhow!("Order size too small after rounding"));
        }

        // Set leverage if specified
        if request.leverage > 1 {
            self.exchange_client
                .update_leverage(
                    request.leverage,
                    &request.asset,
                    request.cross_margin,
                    None
                )
                .await?;
        }

        if request.usd_value < 10.0 {
            return Err(anyhow::anyhow!("Minimum order size is $10"));
        }

        match request.order_type {
            OrderType::Market => {
                // For market orders, use the actual best price from the orderbook
                let market_price = if request.is_buy {
                    best_ask  // Use best ask for buys
                } else {
                    best_bid  // Use best bid for sells
                };

                let order = ClientOrderRequest {
                    asset: request.asset,
                    is_buy: request.is_buy,
                    reduce_only: request.reduce_only,
                    limit_px: market_price,
                    sz: size,
                    cloid: None,
                    order_type: ClientOrder::Limit(ClientLimit {
                        tif: "Ioc".to_string(),
                    }),
                };

                Ok(self.exchange_client.order(order, None).await?)
            }
            
            OrderType::Limit => {
                let price = request.price.expect("Limit orders require a price");
                
                let order = ClientOrderRequest {
                    asset: request.asset,
                    is_buy: request.is_buy,
                    reduce_only: request.reduce_only,
                    limit_px: price,
                    sz: size,
                    cloid: None,
                    order_type: ClientOrder::Limit(ClientLimit {
                        tif: "Gtc".to_string(), // Good-til-Cancelled for limit orders
                    }),
                };

                Ok(self.exchange_client.order(order, None).await?)
            }
        }
    }

    pub async fn get_positions(&self) -> Result<Vec<Position>> {
        let state = self.info_client.user_state(self.exchange_client.wallet.address()).await?;
        
        let positions = state
            .asset_positions
            .iter()
            .filter(|pos| pos.position.szi.parse::<f64>().unwrap_or(0.0) != 0.0)
            .filter_map(|pos| Position::from_position_data(&pos.position).ok())
            .collect();
            
        Ok(positions)
    }

    pub async fn get_open_orders(&self) -> Result<Vec<OpenOrder>> {
        // Get user state using the wallet address
        let address = self.exchange_client.wallet.address();
        
        // Get open orders directly from info client
        let open_orders = self.info_client.open_orders(address).await?;
        
        // Convert to our OpenOrder struct
        let orders = open_orders
            .into_iter()
            .map(|order| OpenOrder {
                asset: order.coin,
                price: order.limit_px.parse().unwrap_or(0.0),
                size: order.sz.parse().unwrap_or(0.0),
                side: order.side,
                order_id: order.oid as u64,
                timestamp: order.timestamp,
            })
            .collect();
            
        Ok(orders)
    }
}

#[derive(Debug)]
pub struct OpenOrder {
    pub asset: String,
    pub price: f64,
    pub size: f64,
    pub side: String,
    pub order_id: u64,
    pub timestamp: u64,
}
