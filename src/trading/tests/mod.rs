#[cfg(test)]
mod dydx_tests {
    use crate::trading::wallet::WalletManager;
    use crate::trading::init_logging;
    use anyhow::Result;
    use tracing::{info, debug, error};
    use std::time::Duration;

    #[tokio::test]
    async fn test_dydx_positions() -> Result<()> {
        init_logging();
        info!("Starting dYdX positions test");
        
        let mut wallet_manager = WalletManager::new().await?;
        info!("WalletManager initialized");
        
        wallet_manager.init_dydx_client().await?;
        info!("dYdX client initialized");
            
        if let Some(dydx_service) = wallet_manager.get_dydx_service() {
            if let Some(dydx_wallet) = wallet_manager.get_dydx_wallet() {
                if let Ok(account) = dydx_wallet.account_offline(0) {
                    let subaccount = account.subaccount(0)?;
                    debug!("Using subaccount: {:?}", subaccount);
                    
                    // Add retry logic with delay
                    for attempt in 1..=3 {
                        info!("Attempt {} to fetch positions", attempt);
                        
                        let all_positions_result = dydx_service.indexer_client
                            .accounts()
                            .list_parent_positions(&subaccount.parent(), None)
                            .await;
                        
                        match &all_positions_result {
                            Ok(positions) => {
                                info!("Successfully fetched positions");
                                info!("Number of positions: {}", positions.len());
                                info!("Raw positions response: {:?}", positions);
                                break;
                            }
                            Err(e) => {
                                error!("Error getting positions (attempt {}): {:?}", attempt, e);
                                if attempt < 3 {
                                    tokio::time::sleep(Duration::from_secs(2)).await;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_dydx_orders() -> Result<()> {
        init_logging();
        info!("Starting dYdX orders test");
        
        let mut wallet_manager = WalletManager::new().await?;
        info!("WalletManager initialized");
        
        wallet_manager.init_dydx_client().await?;
        info!("dYdX client initialized");
        
        if let Some(dydx_service) = wallet_manager.get_dydx_service() {
            if let Some(dydx_wallet) = wallet_manager.get_dydx_wallet() {
                if let Ok(account) = dydx_wallet.account_offline(0) {
                    let subaccount = account.subaccount(0)?;
                    debug!("Using subaccount: {:?}", subaccount);
                    
                    // Get all orders with no filters at all
                    info!("Attempting to get all orders without any filters");
                    let orders_result = dydx_service.indexer_client
                        .accounts()
                        .list_parent_orders(&subaccount.parent(), None)  // Use parent endpoint
                        .await;
                    
                    match &orders_result {
                        Ok(orders) => {
                            info!("Complete raw orders response: {:#?}", orders);
                            info!("Number of orders: {}", orders.len());
                        }
                        Err(e) => {
                            info!("Error getting orders: {:?}", e);
                            info!("Error details: {:#?}", e);
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
