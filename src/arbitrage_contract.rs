use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount};
use mango::*;
use jupiter_core::*;
use raydium_amm::*;

declare_id!("atXVy7bPRA1j81moNmmhhioKtAAu8XxzUDjN9L8ZUmW");

#[program]
pub mod arbitrage_contract {
    use super::*;

    pub struct ArbitrageContract {
        pub mango_program: Program<Mango>,
        pub jupiter_program: Program<Jupiter>,
        pub raydium_program: Program<Raydium>,
    }

    #[derive(Accounts)]
    pub struct Initialize {
        #[account(mut)]
        pub user: Signer<'info>,
        pub system_program: Program<'info, System>,
    }

    #[derive(Accounts)]
    pub struct TryArbitrage<'info> {
        #[account(mut)]
        pub user: Signer<'info>,
        #[account(mut)]
        pub token_a_account: Account<'info, TokenAccount>,
        #[account(mut)]
        pub token_b_account: Account<'info, TokenAccount>,
        #[account(mut)]
        pub mango_account: AccountInfo<'info>,
        pub mango_program: Program<'info, Mango>,
        pub jupiter_program: Program<'info, Jupiter>,
        pub raydium_program: Program<'info, Raydium>,
        pub token_program: Program<'info, Token>,
        #[account(mut)]
        pub profit_destination: Account<'info, TokenAccount>,
    }

    #[error_code]
    pub enum ErrorCode {
        #[msg("Error in calculation")]
        CalculationError,
        #[msg("Insufficient profit for arbitrage")]
        InsufficientProfit,
        #[msg("Invalid token account")]
        InvalidTokenAccount,
        #[msg("Slippage tolerance exceeded")]
        SlippageExceeded,
    }

    impl ArbitrageContract {
        pub fn try_arbitrage(ctx: Context<TryArbitrage>) -> Result<()> {
            // 1. Get prices from both DEXes
            let jupiter_price = Self::get_jupiter_price(&ctx.accounts)?;
            let raydium_price = Self::get_raydium_price(&ctx.accounts)?;

            // 2. Check if arbitrage is profitable (including fees)
            if Self::is_profitable(jupiter_price, raydium_price, amount) {
                // 3. Execute flash loan from Mango
                
                Self::execute_flash_loan(ctx.accounts, amount, token_a, |borrowed_sol| {
                    // 1. Convert borrowed SOL to token_a using Jupiter
                    let initial_token_amount = Self::swap_sol_to_token(
                        borrowed_sol,
                        token_a,
                        &ctx.accounts.jupiter_program
                    )?;

                    // 2. Execute the arbitrage between token_a and token_b
                    let profit_in_token = if jupiter_price > raydium_price {
                        Self::swap_on_raydium(initial_token_amount, token_a, token_b)?;
                        Self::swap_on_jupiter(initial_token_amount, token_b, token_a)?
                    } else {
                        Self::swap_on_jupiter(initial_token_amount, token_a, token_b)?;
                        Self::swap_on_raydium(initial_token_amount, token_b, token_a)?
                    };

                    // 3. Convert profit back to SOL for loan repayment
                    Self::swap_token_to_sol(
                        profit_in_token,
                        token_a,
                        &ctx.accounts.jupiter_program
                    )?;

                    Ok(())
                })?;
            }

            // After successful arbitrage, transfer profits
            if profit > 0 {
                // Transfer the profit to your wallet
                token::transfer(
                    CpiContext::new(
                        ctx.accounts.token_program.to_account_info(),
                        token::Transfer {
                            from: ctx.accounts.token_a_account.to_account_info(),
                            to: ctx.accounts.profit_destination.to_account_info(),
                            authority: ctx.accounts.user.to_account_info(),
                        },
                    ),
                    profit,
                )?;
            }

            Ok(())
        }

        fn get_jupiter_price(accounts: &TryArbitrage) -> Result<u64> {
            // Create a quote request to Jupiter
            let quote_request = jupiter_core::QuoteRequest {
                input_mint: token_a,
                output_mint: token_b,
                amount,
                slippage_bps: 300, // 0.3% slippage
                only_direct_routes: true, // For faster price checks
            };

            // Get the quote from Jupiter
            let quote = jupiter_core::quote(
                &accounts.jupiter_program,
                &quote_request,
            )?;

            // Extract the output amount from the quote
            let output_amount = quote.out_amount;

            // Calculate the effective price (output amount per input token)
            let price = (output_amount)
                .checked_mul(PRICE_PRECISION)
                .ok_or(ErrorCode::CalculationError)?
                .checked_div(amount)
                .ok_or(ErrorCode::CalculationError)?;

            Ok(price)
        }

        fn get_raydium_price(accounts: &TryArbitrage) -> Result<u64> {
            // Get the pool state for the token pair
            let pool = raydium_amm::state::AmmInfo::load(
                &accounts.raydium_program,
                token_a,
                token_b,
            )?;

            // Get the current reserves from the pool
            let (reserve_a, reserve_b) = (
                pool.token_a_reserve,
                pool.token_b_reserve,
            );

            // Calculate the output amount using the constant product formula (x * y = k)
            // new_reserve_a = reserve_a + amount
            // new_reserve_b = k / new_reserve_a
            // output_amount = reserve_b - new_reserve_b
            let k = reserve_a
                .checked_mul(reserve_b)
                .ok_or(ErrorCode::CalculationError)?;
            
            let new_reserve_a = reserve_a
                .checked_add(amount)
                .ok_or(ErrorCode::CalculationError)?;
            
            let new_reserve_b = k
                .checked_div(new_reserve_a)
                .ok_or(ErrorCode::CalculationError)?;
            
            let output_amount = reserve_b
                .checked_sub(new_reserve_b)
                .ok_or(ErrorCode::CalculationError)?;

            // Apply Raydium's fee (0.25% typical fee)
            let fee_numerator = 25;
            let fee_denominator = 10000;
            let output_after_fees = output_amount
                .checked_mul(fee_denominator - fee_numerator)
                .ok_or(ErrorCode::CalculationError)?
                .checked_div(fee_denominator)
                .ok_or(ErrorCode::CalculationError)?;

            // Calculate the effective price (output amount per input token)
            let price = output_after_fees
                .checked_mul(PRICE_PRECISION)
                .ok_or(ErrorCode::CalculationError)?
                .checked_div(amount)
                .ok_or(ErrorCode::CalculationError)?;

            Ok(price)
        }

        fn is_profitable(price_a: u64, price_b: u64, amount: u64) -> bool {
            // Updated to account for additional Jupiter swap fees
            let mango_fee = Self::calculate_mango_fee(amount);
            let dex_fees = Self::calculate_dex_fees(amount);
            let jupiter_conversion_fees = Self::calculate_jupiter_conversion_fees(amount);
            let gas_cost = Self::estimate_gas_cost();
            
            let potential_profit = (price_a.max(price_b) - price_a.min(price_b)) * amount;
            potential_profit > (mango_fee + dex_fees + jupiter_conversion_fees + gas_cost)
        }

        fn calculate_mango_fee(amount: u64) -> u64 {
            // Mango flash loan fee is typically 0.2%
            amount
                .checked_mul(20)
                .unwrap_or(0)
                .checked_div(10000)
                .unwrap_or(0)
        }

        fn calculate_dex_fees(amount: u64) -> u64 {
            // Jupiter fee: 0.3%
            let jupiter_fee = amount
                .checked_mul(30)
                .unwrap_or(0)
                .checked_div(10000)
                .unwrap_or(0);
            
            // Raydium fee: 0.25%
            let raydium_fee = amount
                .checked_mul(25)
                .unwrap_or(0)
                .checked_div(10000)
                .unwrap_or(0);
            
            // Return total fees for both swaps
            jupiter_fee.checked_add(raydium_fee).unwrap_or(0)
        }

        fn estimate_gas_cost() -> u64 {
            // Estimate gas cost in lamports
            // Flash loan + 2 swaps typically costs around 0.01 SOL
            // 1 SOL = 1_000_000_000 lamports
            // 0.01 SOL = 10_000_000 lamports
            10_000_000
        }

        fn execute_flash_loan<F>(accounts: &TryArbitrage, amount: u64, token: Pubkey, operation: F) -> Result<()>
        where F: FnOnce(u64) -> Result<()> {
            // Implement Mango flash loan logic
            // 1. Borrow funds from Mango
            // 2. Pass the borrowed amount to the operation function
            // 3. Repay the loan with fees
            let borrowed_funds = amount; // Placeholder for borrowed funds

            operation(borrowed_funds)?;

            // Repay the loan - placeholder logic
            let repay_amount = amount + Self::calculate_mango_fee(amount);
            token::transfer(
                CpiContext::new(accounts.token_program.to_account_info(), token::Transfer {
                    from: accounts.token_a_account.to_account_info(),
                    to: accounts.mango_account.to_account_info(),
                    authority: accounts.user.to_account_info(),
                }),
                repay_amount,
            )?;

            Ok(())
        }

        fn swap_on_jupiter(amount: u64, token_a: Pubkey, token_b: Pubkey) -> Result<()> {
            // Create swap instruction
            let swap_instruction = jupiter_core::SwapInstruction {
                input_mint: token_a,
                output_mint: token_b,
                amount,
                slippage_bps: 300, // 0.01% slippage tolerance
                platform_fee_bps: 0, // No additional platform fee
            };

            // Execute the swap through Jupiter's CPI
            jupiter_core::swap(
                CpiContext::new(
                    ctx.accounts.jupiter_program.to_account_info(),
                    jupiter_core::Swap {
                        user: ctx.accounts.user.to_account_info(),
                        user_token_account_a: ctx.accounts.token_a_account.to_account_info(),
                        user_token_account_b: ctx.accounts.token_b_account.to_account_info(),
                        token_program: ctx.accounts.token_program.to_account_info(),
                    },
                ),
                swap_instruction,
            )?;

            Ok(())
        }

        fn swap_on_raydium(amount: u64, token_a: Pubkey, token_b: Pubkey) -> Result<()> {
            // Get pool state and authority
            let pool = raydium_amm::state::AmmInfo::load(
                &ctx.accounts.raydium_program,
                token_a,
                token_b,
            )?;
            
            let pool_authority = Pubkey::find_program_address(
                &[pool.to_account_info().key.as_ref()],
                ctx.accounts.raydium_program.key,
            ).0;

            // Create swap instruction
            let swap_instruction = raydium_amm::instruction::Swap {
                amount_in: amount,
                minimum_amount_out: amount
                    .checked_mul(995) // 0.5% slippage
                    .ok_or(ErrorCode::CalculationError)?
                    .checked_div(1000)
                    .ok_or(ErrorCode::CalculationError)?,
            };

            // Execute the swap through Raydium's CPI
            raydium_amm::swap(
                CpiContext::new(
                    ctx.accounts.raydium_program.to_account_info(),
                    raydium_amm::Swap {
                        amm: pool.to_account_info(),
                        authority: pool_authority,
                        user: ctx.accounts.user.to_account_info(),
                        source_token: ctx.accounts.token_a_account.to_account_info(),
                        destination_token: ctx.accounts.token_b_account.to_account_info(),
                        token_program: ctx.accounts.token_program.to_account_info(),
                    },
                ),
                swap_instruction,
            )?;

            Ok(())
        }

        // New helper functions
        fn swap_sol_to_token(
            sol_amount: u64,
            token: Pubkey,
            jupiter_program: &Program<Jupiter>
        ) -> Result<u64> {
            let wsol_mint = "So11111111111111111111111111111111111111112";
            let swap_instruction = jupiter_core::SwapInstruction {
                input_mint: Pubkey::from_str(wsol_mint)?,
                output_mint: token,
                amount: sol_amount,
                slippage_bps: 300,
                platform_fee_bps: 0,
            };

            // Execute swap through Jupiter
            let result = jupiter_core::swap(
                CpiContext::new(
                    jupiter_program.to_account_info(),
                    jupiter_core::Swap {
                        user: ctx.accounts.user.to_account_info(),
                        user_token_account_a: ctx.accounts.token_a_account.to_account_info(),
                        user_token_account_b: ctx.accounts.token_b_account.to_account_info(),
                        token_program: ctx.accounts.token_program.to_account_info(),
                    },
                ),
                swap_instruction,
            )?;

            Ok(result.amount_out)
        }

        fn swap_token_to_sol(
            token_amount: u64,
            token: Pubkey,
            jupiter_program: &Program<Jupiter>
        ) -> Result<u64> {
            let wsol_mint = "So11111111111111111111111111111111111111112";
            let swap_instruction = jupiter_core::SwapInstruction {
                input_mint: token,
                output_mint: Pubkey::from_str(wsol_mint)?,
                amount: token_amount,
                slippage_bps: 300,
                platform_fee_bps: 0,
            };

            // Execute swap through Jupiter
            let result = jupiter_core::swap(
                CpiContext::new(
                    jupiter_program.to_account_info(),
                    jupiter_core::Swap {
                        user: ctx.accounts.user.to_account_info(),
                        user_token_account_a: ctx.accounts.token_a_account.to_account_info(),
                        user_token_account_b: ctx.accounts.token_b_account.to_account_info(),
                        token_program: ctx.accounts.token_program.to_account_info(),
                    },
                ),
                swap_instruction,
            )?;

            // Return the amount of SOL received
            Ok(result.amount_out)
        }

        fn calculate_jupiter_conversion_fees(amount: u64) -> u64 {
            // Jupiter fee for SOL -> token and token -> SOL (0.3% each way)
            amount
                .checked_mul(60) // 0.6% total
                .unwrap_or(0)
                .checked_div(10000)
                .unwrap_or(0)
        }
    }
}
