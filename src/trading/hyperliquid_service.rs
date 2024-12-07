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

pub struct HyperliquidService {
    exchange_client: ExchangeClient,
    info_client: InfoClient,
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

        let info_client = InfoClient::new(
            None,
            Some(BaseUrl::Mainnet)
        ).await?;

        Ok(Self { 
            exchange_client,
            info_client 
        })
    }

    pub async fn place_trade(&self, request: TradeRequest) -> Result<ExchangeResponseStatus> {
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

        match request.order_type {
            OrderType::Market => {
                // For market orders, we'll use a limit IOC order with a favorable price
                let order = ClientOrderRequest {
                    asset: request.asset,
                    is_buy: request.is_buy,
                    reduce_only: request.reduce_only,
                    limit_px: if request.is_buy { f64::MAX } else { 0.0 }, // Use extreme prices for market orders
                    sz: request.amount,
                    cloid: None,
                    order_type: ClientOrder::Limit(ClientLimit {
                        tif: "Ioc".to_string(), // Immediate-or-Cancel for market orders
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
                    sz: request.amount,
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
}
