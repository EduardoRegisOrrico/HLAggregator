use dydx::{
    node::{NodeClient, NodeConfig, NodeError, OrderTimeInForce, Account, OrderBuilder, OrderId},
    indexer::{IndexerClient, IndexerConfig,PerpetualPositionStatus,ListPositionsOpts},
    indexer::types::{
        Subaccount, OrderSide, OrderType,
        OrderResponseObject,
    },
};
use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::fmt;
use std::error::Error as StdError;
use tracing::error;
use bigdecimal::BigDecimal;
use chrono::{TimeDelta, Utc};
use dydx::node::OrderSide as NodeOrderSide;
use std::str::FromStr;
use dydx::indexer::Ticker;

pub use dydx::indexer::PerpetualPositionResponseObject;
pub use dydx::indexer::{RestConfig, SockConfig};

pub struct DydxService {
    node_client: Arc<Mutex<NodeClient>>,
    pub indexer_client: Arc<IndexerClient>,
    account: Account,
}

#[derive(Debug)]
pub enum DydxServiceError {
    ClientError(NodeError),
    IndexerError(anyhow::Error),
    InvalidParameters(String),
    ParseError(String),
}

impl fmt::Display for DydxServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DydxServiceError::ClientError(e) => write!(f, "Client error: {}", e),
            DydxServiceError::IndexerError(e) => write!(f, "Indexer error: {}", e),
            DydxServiceError::InvalidParameters(s) => write!(f, "Invalid parameters: {}", s),
            DydxServiceError::ParseError(s) => write!(f, "Parse error: {}", s),
        }
    }
}

impl StdError for DydxServiceError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            DydxServiceError::ClientError(e) => Some(e),
            DydxServiceError::IndexerError(e) => Some(e.as_ref()),
            DydxServiceError::InvalidParameters(_) => None,
            DydxServiceError::ParseError(_) => None,
        }
    }
}

impl From<NodeError> for DydxServiceError {
    fn from(error: NodeError) -> Self {
        DydxServiceError::ClientError(error)
    }
}

impl From<anyhow::Error> for DydxServiceError {
    fn from(error: anyhow::Error) -> Self {
        DydxServiceError::IndexerError(error)
    }
}

impl From<bigdecimal::ParseBigDecimalError> for DydxServiceError {
    fn from(error: bigdecimal::ParseBigDecimalError) -> Self {
        DydxServiceError::ParseError(error.to_string())
    }
}

pub struct TradeRequest {
    pub asset: String,
    pub is_buy: bool,
    pub size: f64,
    pub price: Option<f64>,
    pub order_type: OrderType,
    pub reduce_only: bool,
    pub leverage: f64,
}

impl DydxService {
    pub async fn new(
        node_config: NodeConfig, 
        indexer_config: IndexerConfig,
        account: Account
    ) -> Result<Self, DydxServiceError> {
        let node_client = NodeClient::connect(node_config).await?;
        let indexer_client = IndexerClient::new(indexer_config);
        
        Ok(Self {
            node_client: Arc::new(Mutex::new(node_client)),
            indexer_client: Arc::new(indexer_client),
            account,
        })
    }

    /// Place a new order
    pub async fn place_trade(&mut self, request: TradeRequest, leverage: f64) -> Result<String, DydxServiceError> {
        // Format the asset ticker correctly
        let formatted_ticker = if !request.asset.contains("-USD") {
            format!("{}-USD", request.asset)
        } else {
            request.asset.clone()
        };

        // Log the request details
        let request_details = format!(
            "Sending request to dYdX:\n\
             Formatted Ticker: {}\n\
             Side: {}\n\
             Size: {}\n\
             Price: {}\n\
             Type: {:?}\n\
             URL: https://indexer.dydx.trade/v4/perpetualMarkets?limit=1&ticker={}",
            formatted_ticker,
            if request.is_buy { "Buy" } else { "Sell" },
            request.size,
            request.price.map_or("Market".to_string(), |p| p.to_string()),
            request.order_type,
            formatted_ticker
        );

        // Get market data from indexer using formatted ticker
        let market = self.indexer_client
            .markets()
            .get_perpetual_market(&formatted_ticker.clone().into())
            .await
            .map_err(|e| DydxServiceError::IndexerError(
                anyhow::anyhow!("{}\nRequest details:\n{}", e, request_details)
            ))?;

        // Create subaccount from the account
        let subaccount = self.account.subaccount(0)?;

        self.set_margin_requirements(&subaccount, &formatted_ticker, leverage).await?;
        // Convert side
        let side = if request.is_buy {
            OrderSide::Buy
        } else {
            OrderSide::Sell
        };

        // Convert OrderSide
        let side = match side {
            OrderSide::Buy => NodeOrderSide::Buy,
            OrderSide::Sell => NodeOrderSide::Sell,
        };

        // Convert size and price to BigDecimal
        let size_bd = BigDecimal::from_str(&request.size.to_string())
            .map_err(|e| DydxServiceError::InvalidParameters(format!("Invalid size: {}", e)))?;
        
        // Build the order based on type
        let (_, order) = match request.order_type {
            OrderType::Market => {
                // Get current block height for market orders
                let current_block_height = self.node_client.lock().unwrap()
                    .get_latest_block_height()
                    .await?;

                OrderBuilder::new(market.clone(), subaccount)
                    .market(side, size_bd)
                    .time_in_force(OrderTimeInForce::Ioc)
                    .reduce_only(request.reduce_only)
                    .short_term()
                    .until(current_block_height.ahead(5))
                    .build(rand::random::<u32>())?
            },
            OrderType::Limit => {
                let price = request.price.ok_or_else(|| 
                    DydxServiceError::InvalidParameters("Limit orders require a price".to_string())
                )?;
                
                let price_bd = BigDecimal::from_str(&price.to_string())
                    .map_err(|e| DydxServiceError::InvalidParameters(format!("Invalid price: {}", e)))?;

                OrderBuilder::new(market.clone(), subaccount)
                    .limit(
                        side,
                        price_bd,
                        size_bd
                    )
                    .time_in_force(OrderTimeInForce::Unspecified)
                    .reduce_only(request.reduce_only)
                    .long_term()
                    .until(Utc::now() + TimeDelta::days(28))
                    .build(rand::random::<u32>())?
            },
            unsupported_type => {
                return Err(DydxServiceError::InvalidParameters(
                    format!("Order type {:?} is not yet supported", unsupported_type)
                ));
            }
        };

        // Place the order with timeout
        let tx_hash = tokio::time::timeout(
            std::time::Duration::from_secs(30),  // 30 second timeout
            self.node_client.lock().unwrap().place_order(&mut self.account, order)
        ).await
        .map_err(|_| DydxServiceError::ClientError(NodeError::General(
            anyhow::Error::msg("Order placement timed out after 30 seconds")
        )))??;

        Ok(tx_hash.to_string())
    }

    /// Get all open orders for the account
    pub async fn get_open_orders(&self, subaccount: Subaccount) -> Result<Vec<OrderResponseObject>, DydxServiceError> {
        // Using the indexer client to get orders
        let orders = self.indexer_client
            .accounts()
            .list_parent_orders(&subaccount.parent(), None)
            .await?;

        Ok(orders)
    }

    /// Get all open positions for the account
    pub async fn get_open_positions(&self, subaccount: Subaccount) 
        -> Result<Vec<PerpetualPositionResponseObject>, DydxServiceError> {
        let positions_result = self.indexer_client
            .accounts()
            .list_parent_positions(
                &subaccount.parent(),
                Some(ListPositionsOpts {
                    status: Some(PerpetualPositionStatus::Open),
                    ..Default::default()
                }),
            )
            .await;

        match positions_result {
            Ok(positions) => Ok(positions),
            Err(e) => {
                error!("Error retrieving positions: {:?}", e);
                Err(DydxServiceError::IndexerError(e.into()))
            }
        }
    }

    pub fn update_node_client(&mut self, client: NodeClient) {
        self.node_client = Arc::new(Mutex::new(client));
    }

    pub async fn set_margin_requirements(
        &mut self,
        subaccount: &Subaccount,
        market: &str,
        leverage: f64,
    ) -> Result<(), DydxServiceError> {
        // Convert market string to Ticker
        let ticker = Ticker::from(market);

        // Get the market data first
        let market_data = self.indexer_client
            .markets()
            .get_perpetual_market(&ticker)
            .await
            .map_err(|e| DydxServiceError::IndexerError(e.into()))?;

        // For now, we'll just validate the leverage value
        if leverage < 1.0 {
            return Err(DydxServiceError::InvalidParameters("Leverage must be at least 1x".to_string()));
        }


        Ok(())
    }

    pub async fn cancel_order(&mut self, order_id: String) -> Result<String, DydxServiceError> {
        let mut node_client = self.node_client.lock().unwrap();
        
        let current_block_height = node_client.get_latest_block_height().await?;
        
        let client_id = order_id.parse::<u32>()
            .map_err(|e| DydxServiceError::ParseError(format!("Failed to parse order ID: {}", e)))?;
        
        let subaccount = self.account.subaccount(0)?;
        let subaccount_id = Some(dydx_proto::dydxprotocol::subaccounts::SubaccountId {
            owner: subaccount.address.to_string(),
            number: subaccount.number.value(),
        });

        let order_id = OrderId {
            subaccount_id,
            client_id,
            order_flags: 0,
            clob_pair_id: 0,
        };

        // Cancel the order
        let tx_hash = node_client
            .cancel_order(
                &mut self.account,
                order_id,
                current_block_height.ahead(5),
            )
            .await?;

        Ok(tx_hash.to_string())
    }

    pub async fn close_position(
        &mut self, 
        market: String,
        position_size: f64
    ) -> Result<String, DydxServiceError> {
        // Create a market order in the opposite direction
        let request = TradeRequest {
            asset: market.clone(),
            is_buy: position_size < 0.0, // If short position, need to buy to close
            size: position_size.abs(),
            price: None, // Market order
            order_type: OrderType::Market,
            reduce_only: true,
            leverage: 1.0, // Default leverage for closing
        };

        self.place_trade(request, 1.0).await
    }

    pub async fn cancel_all_orders(&mut self, subaccount: Subaccount) -> Result<Vec<String>, DydxServiceError> {
        let open_orders = self.get_open_orders(subaccount).await?;
        let mut cancelled_tx_hashes = Vec::new();

        for order in open_orders {
            match self.cancel_order(order.client_id.to_string()).await {
                Ok(tx_hash) => cancelled_tx_hashes.push(tx_hash),
                Err(e) => {
                    error!("Failed to cancel order {}: {:?}", order.client_id, e);
                    continue;
                }
            }
        }

        Ok(cancelled_tx_hashes)
    }
}