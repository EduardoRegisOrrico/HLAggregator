use ethers::signers::Signer;
use hyperliquid_rust_sdk::{BaseUrl, InfoClient};
use std::io::{self, Write};
use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use ethers::prelude::*;
use dydx::config::ClientConfig;
use std::sync::Arc;
use serde_json;
use ethers::signers::LocalWallet as EthWallet;
use dydx::node::{NodeClient, Wallet as DydxWallet};
use bip32::{Mnemonic, Language};
use crate::trading::dydx_service::TradeRequest;
use bech32;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{stdin, stdout};
use std::fs::OpenOptions;
use chrono::Local;
use ethers::types::{U256, Address as EthAddress};
use ethers::contract::Contract;
use ethers::providers::{Provider, Http};
use crate::trading::dydx_service::DydxService;
use dydx::indexer::{IndexerConfig, RestConfig, SockConfig};
use dydx::indexer::types::{OrderResponseObject, OrderSide, OrderType};
use dydx::node::OrderTimeInForce;
use num_traits::ToPrimitive;
use dydx::indexer::Usdc;
use rust_decimal::prelude::*;
use bigdecimal::BigDecimal;
use std::str::FromStr;
use dydx::noble::NobleClient;
use dydx::noble::NobleUsdc;
use ethers::abi::Abi;
use tonic::transport::{Channel, ClientTlsConfig};
use tokio::time::Duration;
use ethers::types::Address;
use crate::trading::positions::Position;
use dydx_proto::dydxprotocol::subaccounts::SubaccountId;

const ARBITRUM_RPC: &str = "https://arbitrum.llamarpc.com";
const USDC_ADDRESS: &str = "0xaf88d065e77c8cc2239327c5edb3a432268e5831"; // Arbitrum USDC
const USDC_ABI: &str = r#"[
    {
        "inputs": [
            {"name": "spender", "type": "address"},
            {"name": "amount", "type": "uint256"}
        ],
        "name": "approve",
        "outputs": [{"name": "", "type": "bool"}],
        "stateMutability": "nonpayable",
        "type": "function"
    },
    {
        "inputs": [{"name": "account", "type": "address"}],
        "name": "balanceOf",
        "outputs": [{"name": "", "type": "uint256"}],
        "stateMutability": "view",
        "type": "function"
    },
    {
        "inputs": [
            {"name": "owner", "type": "address"},
            {"name": "spender", "type": "address"}
        ],
        "name": "allowance",
        "outputs": [{"name": "", "type": "uint256"}],
        "stateMutability": "view",
        "type": "function"
    }
]"#;
const TOKEN_MESSENGER_ADDRESS: &str = "0xbd3fa81b58ba92a82136038b25adec7066af3155"; // Arbitrum TokenMessenger
const TOKEN_MESSENGER_ABI: &str = r#"[
    {
        "inputs": [
            {"type": "uint256", "name": "amount"},
            {"type": "uint32", "name": "destinationDomain"},
            {"type": "bytes32", "name": "recipient"},
            {"type": "address", "name": "token"}
        ],
        "name": "depositForBurn",
        "outputs": [{"type": "uint64", "name": "nonce"}],
        "stateMutability": "nonpayable",
        "type": "function"
    }
]"#;
const MESSAGE_TRANSMITTER_ADDRESS: &str = "0xc30362313fbba5cf9163f0bb16a0e01f01a896ca";
const CIRCLE_BRIDGE_ADDRESS: &str = "0x19330d10d9cc8751218eaf51e8885d058642e08a";
const DESTINATION_CALLER: &str = "0x0000000000000000000000000000000000000000";
const DESTINATION_TOKEN_MESSENGER: &str = "0x57d4eaf1091577a6b7d121202afbd2808134f117";
const CIRCLE_BRIDGE_ABI: &str = r#"[
    {
        "inputs": [
            {"type": "uint256", "name": "amount"},
            {"type": "uint32", "name": "destinationDomain"},
            {"type": "bytes32", "name": "mintRecipient"},
            {"type": "address", "name": "burnToken"}
        ],
        "name": "depositForBurn",
        "outputs": [{"type": "uint64", "name": "nonce"}],
        "stateMutability": "nonpayable",
        "type": "function"
    }
]"#;

#[derive(Default)]
pub struct WalletManager {
    eth_wallet: Option<EthWallet>,
    dydx_wallet: Option<DydxWallet>,
    dydx_client: Option<NodeClient>,
    config_path: PathBuf,
    dydx_service: Option<DydxService>,
}

impl WalletManager {
    pub async fn new() -> Result<Self> {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("trading_aggregator");
        fs::create_dir_all(&config_dir)?;
        
        let config_path = config_dir.join("wallet.key");
        
        let mut manager = Self {
            eth_wallet: None,
            dydx_wallet: None,
            dydx_client: None,
            config_path,
            dydx_service: None,
        };

        // Try to load existing wallets
        if manager.config_path.exists() {
            if let Ok(data) = fs::read_to_string(&manager.config_path) {
                if let Ok(wallet_data) = serde_json::from_str::<serde_json::Value>(&data) {
                    // Load ETH wallet
                    if let Some(key) = wallet_data["eth_key"].as_str() {
                        if let Ok(wallet) = EthWallet::from_bytes(&hex::decode(key)?) {
                            manager.eth_wallet = Some(wallet);
                        }
                    }
                    
                    // Just load the dYdX wallet, initialize client later
                    if let Some(mnemonic) = wallet_data["dydx_mnemonic"].as_str() {
                        if let Ok(wallet) = DydxWallet::from_mnemonic(mnemonic) {
                            manager.dydx_wallet = Some(wallet);
                        }
                    }
                }
            }
        }

        if manager.dydx_wallet.is_some() {
            manager.init_dydx_service().await?;
        }

        Ok(manager)
    }

    pub fn get_dydx_service(&self) -> Option<&DydxService> {
        self.dydx_service.as_ref()
    }

    pub fn get_wallet(&self) -> Option<&EthWallet> {
        self.eth_wallet.as_ref()
    }

    pub fn get_dydx_wallet(&self) -> Option<&DydxWallet> {
        self.dydx_wallet.as_ref()
    }

    pub async fn create_eth_wallet(&mut self) -> Result<()> {
        let eth_wallet = EthWallet::new(&mut rand::thread_rng());
        
        // Read existing wallet data or create new
        let mut wallet_data = if self.config_path.exists() {
            serde_json::from_str(&fs::read_to_string(&self.config_path)?)?
        } else {
            serde_json::json!({})
        };

        // Update ETH wallet data
        wallet_data["eth_key"] = serde_json::Value::String(hex::encode(eth_wallet.signer().to_bytes()));
        
        // Save updated wallet data
        fs::write(&self.config_path, serde_json::to_string_pretty(&wallet_data)?)?;
        self.eth_wallet = Some(eth_wallet);
        
        println!("\nETH wallet created successfully");
        Ok(())
    }

    pub async fn create_dydx_wallet(&mut self) -> Result<()> {
        // Load config
        let config = ClientConfig::from_file("./src/bridge_config/mainnet.toml").await?;

        // Generate new mnemonic
        let mnemonic = Mnemonic::random(&mut rand::thread_rng(), Language::English);
        let phrase = mnemonic.phrase();
        
        // Create wallet
        let dydx_wallet = DydxWallet::from_mnemonic(phrase)?;

        // Initialize client
        let dydx_client = NodeClient::connect(config.node).await?;

        // Save wallet data
        let mut wallet_data = if self.config_path.exists() {
            serde_json::from_str(&fs::read_to_string(&self.config_path)?)?
        } else {
            serde_json::json!({})
        };

        wallet_data["dydx_mnemonic"] = serde_json::Value::String(phrase.to_string());
        fs::write(&self.config_path, serde_json::to_string_pretty(&wallet_data)?)?;
        
        self.dydx_wallet = Some(dydx_wallet);
        self.dydx_client = Some(dydx_client);
        
        println!("\n⚠️  IMPORTANT: Please save your dYdX mnemonic phrase securely: {}", phrase);
        
        Ok(())
    }

    pub async fn import_eth_wallet(&mut self) -> Result<()> {
        // Disable raw mode to allow normal input
        disable_raw_mode()?;
        
        print!("Enter ETH private key (hex format): ");
        io::stdout().flush()?;
        
        let mut eth_input = String::new();
        stdin().read_line(&mut eth_input)?;
        
        // Re-enable raw mode for the UI
        enable_raw_mode()?;
        
        let eth_input = eth_input.trim();

        // Validate and process private key
        let private_key = if eth_input.starts_with("0x") {
            &eth_input[2..]
        } else {
            eth_input
        };

        match hex::decode(private_key) {
            Ok(bytes) => {
                match EthWallet::from_bytes(&bytes) {
                    Ok(eth_wallet) => {
                        // Read existing wallet data or create new
                        let mut wallet_data = if self.config_path.exists() {
                            serde_json::from_str(&fs::read_to_string(&self.config_path)?)?
                        } else {
                            serde_json::json!({})
                        };

                        // Update ETH wallet data
                        wallet_data["eth_key"] = serde_json::Value::String(hex::encode(eth_wallet.signer().to_bytes()));
                        
                        // Save updated wallet data
                        fs::write(&self.config_path, serde_json::to_string_pretty(&wallet_data)?)?;
                        self.eth_wallet = Some(eth_wallet);
                        
                        println!("\nETH wallet imported successfully");
                    }
                    Err(e) => println!("\nInvalid private key format: {}", e),
                }
            }
            Err(e) => println!("\nInvalid hex string: {}", e),
        }

        // Small pause to show the result message
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        Ok(())
    }

    pub async fn import_dydx_wallet(&mut self) -> Result<()> {
        // Disable raw mode to allow normal input
        disable_raw_mode()?;
        
        print!("Enter dYdX mnemonic phrase: ");
        io::stdout().flush()?;
        
        let mut mnemonic_input = String::new();
        stdin().read_line(&mut mnemonic_input)?;
        
        // Re-enable raw mode for the UI
        enable_raw_mode()?;
        
        let mnemonic_input = mnemonic_input.trim();

        match DydxWallet::from_mnemonic(mnemonic_input) {
            Ok(dydx_wallet) => {
                // Read existing wallet data
                let mut wallet_data = if self.config_path.exists() {
                    serde_json::from_str(&fs::read_to_string(&self.config_path)?)?
                } else {
                    serde_json::json!({})
                };

                // Update dYdX wallet data
                wallet_data["dydx_mnemonic"] = serde_json::Value::String(mnemonic_input.to_string());
                
                // Save updated wallet data
                fs::write(&self.config_path, serde_json::to_string_pretty(&wallet_data)?)?;
                self.dydx_wallet = Some(dydx_wallet);
                
                println!("\ndYdX wallet imported successfully");
            }
            Err(e) => println!("\nInvalid mnemonic phrase: {}", e),
        }

        // Small pause to show the result message
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        Ok(())
    }

    // Optional: Helper method to create both wallets at once
    pub async fn create_all_wallets(&mut self) -> Result<()> {
        self.create_eth_wallet().await?;
        self.create_dydx_wallet().await?;
        Ok(())
    }

    // Optional: Helper method to import both wallets at once
    pub async fn import_all_wallets(&mut self) -> Result<()> {
        self.import_eth_wallet().await?;
        self.import_dydx_wallet().await?;
        Ok(())
    }

    pub async fn get_wallet_info(&self) -> Result<(Option<String>, Option<String>, f64, f64, f64)> {
        if let Some(wallet) = &self.eth_wallet {
            // Create provider with chain ID
            let provider = Provider::<Http>::try_from(ARBITRUM_RPC)?;
            let chain_id = 42161u64;
            let wallet_with_chain_id = wallet.clone().with_chain_id(chain_id);
            let client = Arc::new(provider);
            let client_with_signer = Arc::new(SignerMiddleware::new(
                client.clone(),
                wallet_with_chain_id,
            ));

            // Get Hyperliquid info
            let info_client = InfoClient::new(None, Some(BaseUrl::Mainnet)).await?;
            let user_state = info_client.user_state(wallet.address()).await?;
            
            let account_value = user_state.margin_summary.account_value.parse::<f64>()?;
            let margin_used = user_state.margin_summary.total_margin_used.parse::<f64>()?;

            // Create USDC contract
            let usdc_address = USDC_ADDRESS.strip_prefix("0x")
                .unwrap_or(USDC_ADDRESS)
                .parse::<Address>()?;
            
            let abi: ethers::abi::Abi = serde_json::from_str(USDC_ABI)?;
            let contract = Contract::new(
                usdc_address,
                abi,
                client_with_signer.clone()
            );

            // Get USDC balance
            let balance: U256 = contract
                .method::<_, U256>("balanceOf", wallet.address())?
                .call()
                .await?;

            let decimal_balance = balance.as_u128() as f64 / 1_000_000.0;
            let address = format!("{:#x}", wallet.address());
            let private_key = fs::read_to_string(&self.config_path)?;
            
            Ok((Some(address), Some(private_key), account_value, margin_used, decimal_balance))
        } else {
            Ok((None, None, 0.0, 0.0, 0.0))
        }
    }

    pub async fn get_dydx_balance(&mut self) -> Result<Option<f64>> {
        if let Some(dydx_service) = &self.dydx_service {
            if let Some(dydx_wallet) = &self.dydx_wallet {
                if let Ok(account) = dydx_wallet.account_offline(0) {
                    if let Ok(subaccount) = account.subaccount(0) {
                        // Use parent subaccount endpoint instead
                        let parent_subaccount_info = dydx_service.indexer_client
                            .accounts()
                            .get_parent_subaccount(&subaccount.parent())
                            .await?;
                        
                        // Return the equity as the balance
                        return Ok(Some(parent_subaccount_info.equity.to_f64().unwrap_or(0.0)));
                    }
                }
            }
        }
        Ok(None)
    }

    pub async fn get_dydx_account_info(&mut self) -> Result<Option<(String, f64, f64)>> {
        if let Some(dydx_wallet) = &self.dydx_wallet {
            if let Some(client) = &mut self.dydx_client {
                if let Ok(account) = dydx_wallet.account_offline(0) {
                    let address = account.address().to_string();
                    let balances = client.get_account_balances(account.address()).await?;
                    let mut usdc_balance = 0.0;
                    
                    for balance in balances {
                        if balance.denom == "ibc/8E27BA2D5493AF5636760E354E46004562C46AB7EC0CC4C1CA14E9E20E2545B5" {
                            usdc_balance = balance.amount.parse::<f64>()? / 1_000_000.0;
                            break;
                        }
                    }

                    // Get account info from client
                    let account_info = client.get_account(account.address()).await?;
                    
                    return Ok(Some((
                        address,
                        usdc_balance,
                        account_info.sequence as f64,
                    )));
                }
            }
        }
        Ok(None)
    }

    pub async fn init_dydx_client(&mut self) -> Result<()> {
        if self.dydx_wallet.is_some() && self.dydx_client.is_none() {
            if let Ok(config) = ClientConfig::from_file("./src/bridge_config/mainnet.toml").await {
                // Clone the config.node for the second use
                let node_config = config.node.clone();
                if let Ok(client) = NodeClient::connect(config.node).await {
                    // Initialize DydxService
                    if let Some(ref dydx_wallet) = self.dydx_wallet {
                        let indexer_config = IndexerConfig {
                            rest: RestConfig {
                                endpoint: "https://indexer.dydx.trade".to_string(),
                            },
                            sock: SockConfig {
                                endpoint: "wss://indexer.dydx.trade/v4/ws".to_string(),
                                timeout: 1000,
                                rate_limit: std::num::NonZeroU32::new(2).unwrap(),
                            },
                        };
                        
                        if let Ok(account) = dydx_wallet.account_offline(0) {
                            let dydx_service = DydxService::new(
                                node_config,
                                indexer_config,
                                account
                            ).await?;
                            self.dydx_service = Some(dydx_service);
                        }
                    }
                    self.dydx_client = Some(client);
                }
            }
        }
        Ok(())
    }

    pub async fn get_dydx_positions(&self) -> Result<Vec<Position>> {
        if let Some(ref dydx_service) = self.dydx_service {
            if let Some(ref dydx_wallet) = self.dydx_wallet {
                if let Ok(account) = dydx_wallet.account_offline(0) {
                    let subaccount = account.subaccount(0);
                    match subaccount {
                        Ok(sub) => {
                            match dydx_service.indexer_client
                                .accounts()
                                .list_parent_positions(
                                    &sub.parent(),
                                    Some(dydx::indexer::ListPositionsOpts {
                                        status: Some(dydx::indexer::PerpetualPositionStatus::Open),
                                        ..Default::default()
                                    }),
                                )
                                .await {
                                    Ok(raw_positions) => {
                                        let positions: Vec<Position> = raw_positions.into_iter()
                                            .filter(|pos| pos.status == dydx::indexer::PerpetualPositionStatus::Open)
                                            .filter_map(|pos| Position::from_dydx_position(&pos).ok())
                                            .collect();
                                        Ok(positions)
                                    }
                                    Err(_) => Ok(Vec::new())
                                }
                        }
                        Err(_) => Ok(Vec::new())
                    }
                } else {
                    Ok(Vec::new())
                }
            } else {
                Ok(Vec::new())
            }
        } else {
            Ok(Vec::new())
        }
    }

    pub async fn get_dydx_orders(&self) -> Result<Vec<OrderResponseObject>> {
        if let Some(ref dydx_service) = self.dydx_service {
            if let Some(ref dydx_wallet) = self.dydx_wallet {
                if let Ok(account) = dydx_wallet.account_offline(0) {
                    return Ok(dydx_service.indexer_client
                        .accounts()
                        .list_parent_orders(
                            &account.subaccount(0)?.parent(),
                            Some(dydx::indexer::ListOrdersOpts {
                                status: Some(dydx::indexer::OrderStatus::Open),
                                ..Default::default()
                            }),
                        )
                        .await?);
                }
            }
        }
        Ok(Vec::new())
    }

    pub async fn place_dydx_order(
        &mut self,
        market: &str,
        side: OrderSide,
        size: f64,
        price: Option<f64>,
        order_type: OrderType,
        time_in_force: OrderTimeInForce,
        leverage: f64,
    ) -> Result<(String, String)> {
        if let Some(ref mut dydx_service) = self.dydx_service {
            let (tx_hash, order_id) = dydx_service.place_trade(TradeRequest {
                asset: market.to_string(),
                is_buy: matches!(side, OrderSide::Buy),
                size,
                price,
                order_type,
                reduce_only: false,
                leverage,
            }, leverage).await?;
            
            // Format order ID as "client_id:clob_pair_id:order_flags:subaccount_id"
            let formatted_order_id = format!(
                "{}:{}:{}:{}",
                order_id.client_id,
                order_id.clob_pair_id,
                order_id.order_flags,
                order_id.subaccount_id.unwrap_or_default().number
            );
            
            Ok((tx_hash, formatted_order_id))
        } else {
            Err(anyhow::anyhow!("dYdX service not initialized"))
        }
    }

    pub async fn init_dydx_service(&mut self) -> Result<()> {
        if let Some(ref dydx_wallet) = self.dydx_wallet {
            let config = ClientConfig::from_file("./src/bridge_config/mainnet.toml").await?;
            
            // Create the configs
            let node_config = config.node.clone();
            let indexer_config = IndexerConfig {
                rest: RestConfig {
                    endpoint: "https://indexer.dydx.trade".to_string(),
                },
                sock: SockConfig {
                    endpoint: "wss://indexer.dydx.trade/v4/ws".to_string(),
                    timeout: 1000,
                    rate_limit: std::num::NonZeroU32::new(2).unwrap(),
                },
            };

            if let Ok(account) = dydx_wallet.account_offline(0) {
                // Create the DydxService with proper config types
                let service = DydxService::new(
                    node_config,
                    indexer_config,
                    account
                ).await?;
                
                self.dydx_service = Some(service);
            }
        }
        Ok(())
    }

    pub async fn bridge_to_dydx(&self, amount: f64) -> Result<()> {
        // Open log file with timestamp
        let log_path = "./logs/bridge.log";
        let mut log_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;

        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        writeln!(log_file, "\n=== Bridge Operation Started at {} ===", timestamp)?;

        if let Some(wallet) = &self.eth_wallet {
            // Log ETH wallet details
            writeln!(log_file, "ETH Wallet Address: {}", wallet.address())?;
            writeln!(log_file, "Amount to Bridge: {} USDC", amount)?;
            
            // Setup provider and wallet with chain ID
            let provider = Provider::<Http>::try_from(ARBITRUM_RPC)?;
            let chain_id = 42161u64;
            let wallet_with_chain_id = wallet.clone().with_chain_id(chain_id);
            let client = Arc::new(provider);
            let client_with_signer = Arc::new(SignerMiddleware::new(
                client.clone(),
                wallet_with_chain_id,
            ));

            // Initialize Circle Bridge contract
            let circle_bridge_address: Address = CIRCLE_BRIDGE_ADDRESS.parse()?;
            let circle_bridge_abi: ethers::abi::Abi = serde_json::from_str(CIRCLE_BRIDGE_ABI)?;
            let circle_bridge_contract = Contract::new(
                circle_bridge_address,
                circle_bridge_abi,
                client_with_signer.clone()
            );

            // Initialize USDC contract
            let usdc_address: Address = USDC_ADDRESS.parse()?;
            let usdc_abi: ethers::abi::Abi = serde_json::from_str(USDC_ABI)?;
            let usdc_contract = Contract::new(
                usdc_address,
                usdc_abi,
                client_with_signer
            );

            writeln!(log_file, "Connected to Arbitrum RPC: {}", ARBITRUM_RPC)?;
            writeln!(log_file, "USDC Contract Address: {}", USDC_ADDRESS)?;
            writeln!(log_file, "Circle Bridge Address: {}", CIRCLE_BRIDGE_ADDRESS)?;

            // Convert and log amount details
            let amount_in_wei = U256::from((amount * 1_000_000.0) as u64);
            writeln!(log_file, "Amount in USDC (with 6 decimals): {}", amount_in_wei)?;

            // Check and log allowance
            let allowance: U256 = usdc_contract
                .method::<_, U256>("allowance", (wallet.address(), circle_bridge_address))?
                .call()
                .await?;
            writeln!(log_file, "Current USDC Allowance: {}", allowance)?;

            // Check if allowance is sufficient
            if allowance < amount_in_wei {
                writeln!(log_file, "Insufficient allowance. Current: {}, Required: {}", allowance, amount_in_wei)?;
                writeln!(log_file, "Approving USDC spend...")?;
                let approve_tx = usdc_contract
                    .method::<_, bool>(
                        "approve",
                        (circle_bridge_address, U256::from(2).pow(U256::from(256)) - U256::from(1)),
                    )?
                    .send()
                    .await?
                    .await?;
                writeln!(log_file, "USDC Approval TX: {:?}", approve_tx)?;
                
                // Verify new allowance
                let new_allowance: U256 = usdc_contract
                    .method::<_, U256>("allowance", (wallet.address(), circle_bridge_address))?
                    .call()
                    .await?;
                writeln!(log_file, "New USDC Allowance: {}", new_allowance)?;
            } else {
                writeln!(log_file, "USDC spending already approved")?;
            }

            // Check USDC balance
            let balance: U256 = usdc_contract
                .method::<_, U256>("balanceOf", wallet.address())?
                .call()
                .await?;
            writeln!(log_file, "Current USDC Balance: {}", balance)?;

            if balance < amount_in_wei {
                return Err(anyhow::anyhow!("Insufficient USDC balance. Have: {}, Need: {}", balance, amount_in_wei));
            }

            // Get dYdX recipient details
            let dydx_recipient = if let Some(dydx_wallet) = &self.dydx_wallet {
                if let Ok(account) = dydx_wallet.account_offline(0) {
                    let address_str = account.address().to_string();
                    writeln!(log_file, "dYdX Recipient Address (bech32): {}", address_str)?;
                    
                    let (_, data, _) = bech32::decode(&address_str)?;
                    let address_bytes = bech32::convert_bits(&data, 5, 8, false)?;
                    let mut recipient_bytes = [0u8; 32];
                    recipient_bytes[12..].copy_from_slice(&address_bytes);
                    writeln!(log_file, "Recipient Bytes (hex): 0x{}", hex::encode(&recipient_bytes))?;
                    H256::from(recipient_bytes)
                } else {
                    let err = "Failed to get dYdX account";
                    writeln!(log_file, "Error: {}", err)?;
                    return Err(anyhow::anyhow!(err));
                }
            } else {
                let err = "No dYdX wallet configured";
                writeln!(log_file, "Error: {}", err)?;
                return Err(anyhow::anyhow!(err));
            };

            writeln!(log_file, "Initiating bridge transfer...")?;

            // Log all parameters before sending
            writeln!(log_file, "Transaction Parameters:")?;
            writeln!(log_file, "Amount: {}", amount_in_wei)?;
            writeln!(log_file, "Destination Domain: {}", 4u32)?;
            writeln!(log_file, "Recipient: 0x{}", hex::encode(dydx_recipient.as_bytes()))?;
            writeln!(log_file, "Token Address: {}", usdc_address)?;

            // Estimate gas and send the transaction
            let gas_estimate = circle_bridge_contract
                .method::<_, u64>(
                    "depositForBurn",
                    (
                        amount_in_wei,
                        4u32,
                        dydx_recipient,
                        usdc_address,
                    ),
                )?
                .estimate_gas()
                .await?;

            writeln!(log_file, "Estimated gas: {}", gas_estimate)?;

            let bridge_tx = circle_bridge_contract
                .method::<_, u64>(
                    "depositForBurn",
                    (
                        amount_in_wei,
                        4u32,
                        dydx_recipient,
                        usdc_address,
                    ),
                )?
                .gas(gas_estimate.as_u64() + 50_000) // Add buffer to estimated gas
                .send()
                .await?
                .await?;

            writeln!(log_file, "Bridge transfer completed. TX: {:?}", bridge_tx)?;
            writeln!(log_file, "\nPlease wait 10-15 minutes for the funds to arrive on dYdX")?;
            
            writeln!(log_file, "=== Bridge Operation Completed Successfully ===\n")?;
            Ok(())
        } else {
            let err = "No ETH wallet configured";
            writeln!(log_file, "Error: {}", err)?;
            Err(anyhow::anyhow!(err))
        }
    }

    pub async fn cancel_dydx_order(&mut self, order_id: &str) -> Result<String> {
        if let Some(dydx_service) = &mut self.dydx_service {
            // Parse the order_id string into OrderId components
            let parts: Vec<&str> = order_id.split(':').collect();
            if parts.len() != 4 {
                return Err(anyhow::anyhow!("Invalid order ID format"));
            }
            
            let client_id = parts[0].parse::<u32>()
                .map_err(|_| anyhow::anyhow!("Invalid client ID"))?;
            let clob_pair_id = parts[1].parse::<u32>()
                .map_err(|_| anyhow::anyhow!("Invalid CLOB pair ID"))?;
            let order_flags = parts[2].parse::<u32>()
                .map_err(|_| anyhow::anyhow!("Invalid order flags"))?;
            let subaccount_id = parts[3].parse::<u32>()
                .map_err(|_| anyhow::anyhow!("Invalid subaccount ID"))?;

            // Create the OrderId struct with all required fields
            let order_id = dydx::node::OrderId {
                client_id,
                clob_pair_id,
                order_flags,
                subaccount_id: Some(SubaccountId {
                    owner: self.dydx_wallet
                        .as_ref()
                        .unwrap()
                        .account_offline(0)
                        .unwrap()
                        .address()
                        .to_string(),
                    number: subaccount_id,
                }),
            };

            dydx_service.cancel_order(order_id).await
                .map_err(|e| anyhow::anyhow!("Failed to cancel dYdX order: {}", e))
        } else {
            Err(anyhow::anyhow!("dYdX service not initialized"))
        }
    }

    pub async fn close_dydx_position(&mut self, asset: String, size: f64) -> Result<String> {
        if let Some(dydx_service) = &mut self.dydx_service {
            // Extract just the transaction hash from the tuple
            dydx_service.close_position(asset, size).await
                .map(|(tx_hash, _)| tx_hash)  // Only keep the tx_hash
                .map_err(|e| anyhow::anyhow!("Failed to close dYdX position: {}", e))
        } else {
            Err(anyhow::anyhow!("dYdX service not initialized"))
        }
    }
}