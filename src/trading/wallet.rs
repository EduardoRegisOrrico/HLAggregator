use ethers::signers::{LocalWallet, Signer};
use hyperliquid_rust_sdk::{BaseUrl, InfoClient};
use std::io::{self, Write};
use anyhow::Result;
use std::fs;
use std::path::PathBuf;

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
            let address = wallet.address();
            let info_client = InfoClient::new(None, Some(BaseUrl::Mainnet)).await?;
            let user_state = info_client.user_state(address).await?;
            
            println!("\nWallet Address: {}", address);
            println!("Account Value: {} USD", user_state.margin_summary.account_value);
            println!("Total Margin Used: {} USD", user_state.margin_summary.total_margin_used);
        } else {
            println!("\nNo wallet configured");
        }
        Ok(())
    }

    pub async fn manage_wallets(&mut self) -> Result<()> {
        loop {
            println!("\nCurrent Wallet: {}", self.wallet.as_ref()
                .map(|w| format!("{:#x}", w.address()))
                .unwrap_or_else(|| "No wallet configured".to_string()));
                
            self.display_balance().await?;
            println!("\nWallet Management:");
            println!("1. Create New Wallet");
            println!("2. Import Wallet");
            println!("3. Back to Main Menu");
            print!("\nSelect option (1-3): ");
            io::stdout().flush()?;

            let mut input = String::new();
            io::stdin().read_line(&mut input)?;

            match input.trim() {
                "1" => {
                    print!("Creating a new wallet will replace the existing one. Continue? (y/n): ");
                    io::stdout().flush()?;
                    let mut confirm = String::new();
                    io::stdin().read_line(&mut confirm)?;
                    if confirm.trim().to_lowercase() == "y" {
                        self.create_new_wallet().await?;
                    }
                },
                "2" => {
                    print!("Importing a wallet will replace the existing one. Continue? (y/n): ");
                    io::stdout().flush()?;
                    let mut confirm = String::new();
                    io::stdin().read_line(&mut confirm)?;
                    if confirm.trim().to_lowercase() == "y" {
                        self.import_wallet().await?;
                    }
                },
                "3" => break,
                _ => println!("Invalid option"),
            }
        }
        Ok(())
    }

    pub async fn get_wallet_info(&self) -> Result<(Option<String>, f64, f64)> {
        if let Some(wallet) = &self.wallet {
            let address = format!("{:#x}", wallet.address());
            let info_client = InfoClient::new(None, Some(BaseUrl::Mainnet)).await?;
            let user_state = info_client.user_state(wallet.address()).await?;
            
            // Convert strings to f64
            let account_value = user_state.margin_summary.account_value.parse::<f64>()?;
            let margin_used = user_state.margin_summary.total_margin_used.parse::<f64>()?;
            
            Ok((
                Some(address),
                account_value,
                margin_used
            ))
        } else {
            Ok((None, 0.0, 0.0))
        }
    }

    pub async fn create_new_wallet(&mut self) -> Result<()> {
        let wallet = LocalWallet::new(&mut rand::thread_rng());
        println!("\nNew wallet created!");
        println!("Address: {:#x}", wallet.address());
        println!("Private Key: {}", hex::encode(wallet.signer().to_bytes()));
        
        fs::write(&self.config_path, hex::encode(wallet.signer().to_bytes()))?;
        self.wallet = Some(wallet);
        Ok(())
    }

    pub async fn import_wallet(&mut self) -> Result<()> {
        print!("Enter private key (hex format): ");
        io::stdout().flush()?;
        
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        
        let wallet: LocalWallet = input.trim().parse()?;
        println!("\nWallet imported!");
        println!("Address: {:#x}", wallet.address());
        
        fs::write(&self.config_path, input.trim())?;
        self.wallet = Some(wallet);
        Ok(())
    }

    pub fn get_wallet(&self) -> Option<&LocalWallet> {
        self.wallet.as_ref()
    }
} 