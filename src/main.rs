use tokio;
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, read_keypair_file},
};
use std::str::FromStr;
use std::time::Duration;
use std::collections::HashMap;
use pyth_sdk_solana::state::PriceAccount;
use std::env;
use dotenv::dotenv;


struct ArbitrageMonitor {
    rpc_client: RpcClient,
    wallet: Keypair,
    token_pairs: Vec<TokenPair>,
    min_profit_threshold: f64,
    estimated_gas_cost: u64,
    slippage_tolerance: f64,
}

struct TokenPair {
    token_a: Pubkey,
    token_b: Pubkey,
    loan_amount: u64, // This is the amount of SOL to borrow and also the amount to trade
}

impl ArbitrageMonitor {
    pub fn new(
        rpc_url: &str, 
        wallet_keypair_path: &str,  // Changed parameter name for clarity
    ) -> Self {
        let rpc_client = RpcClient::new(rpc_url.to_string());
        let wallet = read_keypair_file(wallet_keypair_path)
            .expect("Failed to load wallet keypair");

        Self {
            rpc_client,
            wallet,  // This is your Phantom wallet keypair
            token_pairs: Vec::new(),
            min_profit_threshold: 0.5,
            estimated_gas_cost: 5000,
            slippage_tolerance: 0.1,
        }
    }

    pub fn add_token_pair(&mut self, token_a: &str, token_b: &str, amount: u64, loan_amount: u64) {
        let pair = TokenPair {
            token_a: Pubkey::from_str(token_a).expect("Invalid token A address"),
            token_b: Pubkey::from_str(token_b).expect("Invalid token B address"),
            amount_to_trade: amount,
            loan_amount,
        };
        self.token_pairs.push(pair);
    }
    
    async fn monitor_opportunities(&self) {
        loop {
            for pair in &self.token_pairs {
                if let Ok(profitable) = self.check_arbitrage_opportunity(pair).await {
                    if profitable {
                        match self.execute_arbitrage(pair).await {
                            Ok(_) => println!("Successfully executed arbitrage for {:?}-{:?}", 
                                            pair.token_a, pair.token_b),
                            Err(e) => println!("Failed to execute arbitrage: {}", e),
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn check_arbitrage_opportunity(&self, pair: &TokenPair) -> Result<bool, Box<dyn std::error::Error>> {
        let program_id = Pubkey::from_str("Your_Program_ID")?;
        
        // Create instruction to check prices
        let instruction = solana_sdk::instruction::Instruction {
            program_id,
            accounts: vec![
                solana_sdk::instruction::AccountMeta::new_readonly(pair.token_a, false),
                solana_sdk::instruction::AccountMeta::new_readonly(pair.token_b, false),
                // Add Jupiter program account
                solana_sdk::instruction::AccountMeta::new_readonly(
                    Pubkey::from_str("JUP4Fb2cqiRUcaTHdrPC8h2gNsA2ETXiPDD33WcGuJB")?, 
                    false
                ),
                // Add Raydium program account
                solana_sdk::instruction::AccountMeta::new_readonly(
                    Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8")?,
                    false
                ),
            ],
            data: vec![
                0, // Instruction discriminator for price check
                pair.amount_to_trade.to_le_bytes().to_vec(),
            ].concat(),
        };

        // Create transaction
        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let transaction = solana_sdk::transaction::Transaction::new_signed_with_payer(
            &[instruction],
            Some(&self.wallet.pubkey()),
            &[&self.wallet],
            recent_blockhash,
        );

        // Simulate transaction to get prices
        let result = self.rpc_client.simulate_transaction(&transaction)?;
        
        // Parse return data to get prices
        if let Some(return_data) = result.value.return_data {
            let data = base64::decode(return_data.data)?;
            
            // First 8 bytes: Jupiter price
            let jupiter_price = u64::from_le_bytes(data[0..8].try_into()?);
            
            // Next 8 bytes: Raydium price
            let raydium_price = u64::from_le_bytes(data[8..16].try_into()?);
            
            // Calculate potential profit (assuming prices are in the same decimal precision)
            let price_diff = if jupiter_price > raydium_price {
                jupiter_price - raydium_price
            } else {
                raydium_price - jupiter_price
            };
            
            let potential_profit = (price_diff as f64 * pair.amount_to_trade as f64) / 1e9; // Convert to SOL
            
            // Calculate minimum required profit including costs
            let gas_cost_in_usd = self.get_gas_cost_in_usd().await?;
            let required_profit = (pair.amount_to_trade as f64 * self.min_profit_threshold / 100.0) 
                + gas_cost_in_usd 
                + (pair.amount_to_trade as f64 * self.slippage_tolerance / 100.0);

            Ok(potential_profit > required_profit)
        } else {
            Err("No return data from price check simulation".into())
        }
    }

    async fn get_gas_cost_in_usd(&self) -> Result<f64, Box<dyn std::error::Error>> {
        let pyth_sol_usd_account = Pubkey::from_str("H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG")?;
        let account_data = self.rpc_client.get_account_data(&pyth_sol_usd_account)?;
        
        let price_account: PriceAccount = pyth_sdk_solana::state::load_price_account(&account_data)?;
        let price_info = price_account.to_price_feed().get_price_unchecked();
        
        let sol_price = price_info.price as f64 * 10f64.powi(price_info.expo);
        let gas_cost_in_usd = (self.estimated_gas_cost as f64 * sol_price) / 1_000_000_000.0;
        
        Ok(gas_cost_in_usd)
    }

    // // Helper function to parse Pyth price data
    // fn parse_pyth_price(data: &[u8]) -> Result<f64, Box<dyn std::error::Error>> {
    //     // Price is stored at offset 128 in the account data
    //     // This is a simplified version - production code should use proper Pyth SDK
    //     let price_bytes = &data[128..136];
    //     let price = i64::from_le_bytes(price_bytes.try_into()?);
    //     let expo_bytes = &data[136..140];
    //     let expo = i32::from_le_bytes(expo_bytes.try_into()?);
        
    //     // Calculate actual price with exponent
    //     let actual_price = (price as f64) * 10f64.powi(expo);
        
    //     Ok(actual_price)
    // }

    async fn execute_arbitrage(&self, pair: &TokenPair) -> Result<(), Box<dyn std::error::Error>> {
        let program_id = Pubkey::from_str("Your_Program_ID")?;
        
        // Use `loan_amount` directly for swaps
        let sol_borrow_amount = pair.loan_amount;

        // First swap SOL → Token A
        let instruction = solana_sdk::instruction::Instruction {
            program_id,
            accounts: vec![
                solana_sdk::instruction::AccountMeta::new(self.wallet.pubkey(), true),  // Signer
                solana_sdk::instruction::AccountMeta::new(pair.token_a, false),         // Token A account
                solana_sdk::instruction::AccountMeta::new(pair.token_b, false),         // Token B account
                // Add other required accounts based on your program's needs
            ],
            data: vec![
                0,  // Instruction discriminator for arbitrage execution
                sol_borrow_amount.to_le_bytes().to_vec(), // Loan amount used as trade amount
            ].concat(),
        };

        // Remaining logic for creating and sending the transaction...
        let recent_blockhash = self.rpc_client.get_latest_blockhash()?;
        let transaction = solana_sdk::transaction::Transaction::new_signed_with_payer(
            &[instruction],
            Some(&self.wallet.pubkey()),
            &[&self.wallet],
            recent_blockhash,
        );

        let result = self.rpc_client.send_and_confirm_transaction(&transaction)?;
        println!("Arbitrage transaction executed: {}", result);
        
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    // Load environment variables from .env file
    dotenv().ok();
    
    let wallet_keypair_path = "wallet-keypair.json";
    
    let program_id = env::var("SOLANA_PROGRAM_ID")
        .expect("Missing SOLANA_PROGRAM_ID environment variable");
    
    let rpc_url = env::var("SOLANA_RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".to_string());

    let mut monitor = ArbitrageMonitor::new(
        &rpc_url,
        wallet_keypair_path,
    );

    // Add token pairs to monitor using env variables
    monitor.add_token_pair(
        &env::var("BONK_TOKEN_ADDRESS").expect("Missing BONK_TOKEN_ADDRESS"),
        &env::var("GOAT_TOKEN_ADDRESS").expect("Missing GOAT_TOKEN_ADDRESS"),
        env::var("LOAN_AMOUNT")
            .unwrap_or_else(|_| "500000000".to_string())
            .parse()
            .expect("Invalid LOAN_AMOUNT"),
    );

    // Start the monitoring process
    monitor.monitor_opportunities().await;
} 

// Borrow SOL from Mango
// SOL → BONK
// BONK → GOAT
// GOAT → BONK (back to BONK!)
// BONK → SOL