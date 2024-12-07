use ethers::signers::{LocalWallet, Signer};
use hyperliquid_rust_sdk::{BaseUrl, InfoClient, ExchangeClient};
use std::io::{self, Write};
use anyhow::Result;
use std::fs;
use std::path::PathBuf;
use crossterm::terminal;
use ethers::prelude::*;
use std::sync::Arc;
use serde_json;

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
    wallet: Option<LocalWallet>,
    config_path: PathBuf,
}

impl WalletManager {
    pub fn new() -> Result<Self> {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("hl_aggregator");
        fs::create_dir_all(&config_dir)?;
        
        let config_path = config_dir.join("wallet.key");
        
        // Try to load existing wallet
        let wallet = if config_path.exists() {
            let key = fs::read_to_string(&config_path)?;
            Some(key.trim().parse()?)
        } else {
            None
        };

        Ok(Self { wallet, config_path })
    }

    pub async fn display_balance(&self) -> Result<()> {
        if let Some(wallet) = &self.wallet {
            let address = format!("{:#X}", wallet.address());
            let info_client = InfoClient::new(None, Some(BaseUrl::Mainnet)).await?;
            let user_state = info_client.user_state(wallet.address()).await?;
            
            // Create contract instance with proper parsing
            let provider = Provider::<Http>::try_from(ARBITRUM_RPC)?;
            let client = Arc::new(provider);
            
            // Parse ABI and create contract
            let abi: ethers::abi::Abi = serde_json::from_str(USDC_ABI)?;
            let contract = Contract::new(
                USDC_ADDRESS.parse::<Address>()?,
                abi,
                client
            );
            
            let balance: U256 = contract
                .method("balanceOf", wallet.address())?
                .call()
                .await?;
            
            println!("\nWallet Address: {}", address);
            println!("USDC Balance: ${:.2} USD", balance.as_u128() as f64 / 1_000_000.0);
            println!("Account Value: ${:.2} USD", 
                user_state.margin_summary.account_value.parse::<f64>().unwrap_or(0.0));
            println!("Total Margin Used: ${:.2} USD", 
                user_state.margin_summary.total_margin_used.parse::<f64>().unwrap_or(0.0));
        } else {
            println!("\nNo wallet configured");
        }
        Ok(())
    }

    pub async fn manage_wallets(&mut self) -> Result<()> {
        loop {
            print!("\x1B[2J\x1B[1;1H");
            io::stdout().flush()?;

            println!("┌{:─^133}┐", " Wallet Management ");
            
            // Display wallet status in a box
            println!("┌{:─^133}┐", "Wallet Status");
            
            if let Some(wallet) = &self.wallet {
                let address = format!("{:#X}", wallet.address());
                let info_client = InfoClient::new(None, Some(BaseUrl::Mainnet)).await?;
                let user_state = info_client.user_state(wallet.address()).await?;
                
                // Get Arbitrum USDC balance
                let provider = Provider::<Http>::try_from(ARBITRUM_RPC)?;
                let client = Arc::new(provider);
                
                // Parse ABI and create contract
                let abi: ethers::abi::Abi = serde_json::from_str(USDC_ABI)?;
                let contract = Contract::new(
                    USDC_ADDRESS.parse::<Address>()?,
                    abi,
                    client
                );
                
                let balance: U256 = contract
                    .method("balanceOf", wallet.address())?
                    .call()
                    .await?;
                
                println!("│ Current Wallet: {}", address);
                println!("│ USDC Balance: ${:.2}", balance.as_u128() as f64 / 1_000_000.0);
                println!("│ Account Value: ${:.2}", 
                    user_state.margin_summary.account_value.parse::<f64>().unwrap_or(0.0));
                println!("│ Total Margin Used: ${:.2}", 
                    user_state.margin_summary.total_margin_used.parse::<f64>().unwrap_or(0.0));
            } else {
                println!("│ No wallet configured");
            }
            
            println!("└{:─^133}┘", "");
            println!("\nOptions:");
            println!("1. Create New Wallet");
            println!("2. Import Existing Wallet");
            println!("3. Back to Main Menu");
            
            print!("\nEnter choice (1-3): ");
            io::stdout().flush()?;
            
            let mut choice = String::new();
            io::stdin().read_line(&mut choice)?;
            
            match choice.trim() {
                "1" => self.create_new_wallet().await?,
                "2" => self.import_wallet().await?,
                "3" => break,
                _ => {
                    println!("Invalid choice!");
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        }
        Ok(())
    }

    pub async fn get_wallet_info(&self) -> Result<(Option<String>, Option<String>, f64, f64, f64)> {
        if let Some(wallet) = &self.wallet {
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

    pub async fn create_new_wallet(&mut self) -> Result<()> {
        let wallet = LocalWallet::new(&mut rand::thread_rng());
        
        fs::write(&self.config_path, hex::encode(wallet.signer().to_bytes()))?;
        self.wallet = Some(wallet);
        Ok(())
    }

    pub async fn import_wallet(&mut self) -> Result<()> {
        // Disable raw mode for input
        terminal::disable_raw_mode()?;
        print!("\x1B[2J\x1B[1;1H");  // Clear screen
        
        print!("Enter private key (hex format): ");
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        // Trim whitespace and "0x" prefix if present
        let key = input.trim().trim_start_matches("0x");
        
        // Validate and parse the private key
        match LocalWallet::from_bytes(&hex::decode(key)?) {
            Ok(wallet) => {
                // Save the private key and set wallet
                fs::write(&self.config_path, key)?;
                self.wallet = Some(wallet);
                
                // Re-enable raw mode
                terminal::enable_raw_mode()?;
                Ok(())
            },
            Err(e) => {
                println!("\nError importing wallet: {}", e);
                println!("\nPress Enter to continue...");
                let mut temp = String::new();
                io::stdin().read_line(&mut temp)?;
                
                // Re-enable raw mode
                terminal::enable_raw_mode()?;
                Err(anyhow::anyhow!("Invalid private key"))
            }
        }
    }

    pub async fn deposit_usdc_to_hyperliquid(&mut self, amount: f64) -> Result<()> {
        if let Some(wallet) = &self.wallet {
            let exchange_client = ExchangeClient::new(
                None,
                wallet.clone(),
                Some(BaseUrl::Mainnet),
                None,
                None
            ).await?;

            // Transfer USDC to perp trading
            match exchange_client.class_transfer(amount, true, None).await {
                Ok(_) => {
                    println!("Successfully deposited ${:.2} USDC to Hyperliquid", amount);
                    Ok(())
                },
                Err(e) => {
                    println!("Failed to deposit USDC: {}", e);
                    Err(anyhow::anyhow!("Failed to deposit USDC: {}", e))
                }
            }
        } else {
            Err(anyhow::anyhow!("No wallet configured"))
        }
    }

    pub fn get_wallet(&self) -> Option<&LocalWallet> {
        self.wallet.as_ref()
    }
} 