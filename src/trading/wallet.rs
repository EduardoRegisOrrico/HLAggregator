use ethers::signers::{LocalWallet, Signer};
use hyperliquid_rust_sdk::{BaseUrl, InfoClient, ExchangeClient};
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
use bech32;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{stdin, stdout};
use std::fs::OpenOptions;
use chrono::Local;

const ARBITRUM_RPC: &str = "https://arb1.arbitrum.io/rpc";
const USDC_ADDRESS: &str = "0xaf88d065e77c8cC2239327C5EDb3A432268e5831"; // Arbitrum USDC
const USDC_ABI: &str = r#"[{
    "inputs":[{"internalType":"address","name":"account","type":"address"}],
    "name":"balanceOf",
    "outputs":[{"internalType":"uint256","name":"","type":"uint256"}],
    "stateMutability":"view",
    "type":"function"
}]"#;

#[derive(Default)]
pub struct WalletManager {
    eth_wallet: Option<EthWallet>,
    dydx_wallet: Option<DydxWallet>, 
    dydx_client: Option<NodeClient>,
    config_path: PathBuf,
}

impl WalletManager {
    pub fn new() -> Result<Self> {
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
                    
                    // Load dYdX wallet
                    if let Some(mnemonic) = wallet_data["dydx_mnemonic"].as_str() {
                        if let Ok(wallet) = DydxWallet::from_mnemonic(mnemonic) {
                            manager.dydx_wallet = Some(wallet);
                        }
                    }
                }
            }
        }

        Ok(manager)
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
        let config = ClientConfig::from_file("./src/config/mainnet.toml").await?;

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
            let address = format!("{:#x}", wallet.address());
            let private_key = fs::read_to_string(&self.config_path)?;
            
            // Get Hyperliquid info
            let info_client = InfoClient::new(None, Some(BaseUrl::Mainnet)).await?;
            let user_state = info_client.user_state(wallet.address()).await?;
            
            let account_value = user_state.margin_summary.account_value.parse::<f64>()?;
            let margin_used = user_state.margin_summary.total_margin_used.parse::<f64>()?;
            
            // Add retry logic and better error handling for RPC connection
            let provider = Provider::<Http>::try_from(ARBITRUM_RPC)
                .map_err(|e| anyhow::anyhow!("Failed to connect to Arbitrum RPC: {}", e))?;
            let client = Arc::new(provider);
            
            // Add timeout for the RPC call
            let abi: ethers::abi::Abi = serde_json::from_str(USDC_ABI)?;
            let contract = Contract::new(
                USDC_ADDRESS.parse::<Address>()?,
                abi,
                client.clone()
            );

            // Add more detailed error handling for the balance call
            let balance: U256 = match contract.method("balanceOf", wallet.address()) {
                Ok(method) => match method.call().await {
                    Ok(bal) => bal,
                    Err(e) => {
                        println!("Error calling balanceOf: {}", e);
                        return Err(anyhow::anyhow!("Failed to get USDC balance: {}", e));
                    }
                },
                Err(e) => {
                    println!("Error creating balanceOf method call: {}", e);
                    return Err(anyhow::anyhow!("Failed to create balance check: {}", e));
                }
            };

            let decimal_balance = balance.as_u128() as f64 / 1_000_000.0;
            
            Ok((Some(address), Some(private_key), account_value, margin_used, decimal_balance))
        } else {
            Ok((None, None, 0.0, 0.0, 0.0))
        }
    }
}