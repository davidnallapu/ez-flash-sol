# Flash Loan Easy Solution

A Solana program demonstrating how to implement flash loans on the Solana blockchain. This program allows users to borrow tokens temporarily within a single transaction, provided they repay the loan amount plus any fees before the transaction completes.

## Prerequisites

- Rust and Cargo installed
- Solana CLI tools
- Anchor framework
- Node.js and npm (for client interactions)

## Installation

1. Clone the repository:
```bash
git clone [your-repository-url]
cd flash_easy_sol
```

2. Install dependencies:
```bash
cargo build
```

## Setup Instructions

### 1. Generate a New Keypair

```bash
solana-keygen new
```
This will create a new keypair in the default location (`~/.config/solana/id.json`)

### 2. Set Solana Configuration to Devnet

```bash
solana config set --url devnet
```

### 3. Get Devnet SOL

```bash
solana airdrop 2
```

### 4. Build and Deploy the Program

```bash
anchor build
```

After building, you'll find your program ID in `target/deploy/flash_easy_sol-keypair.json`

### 5. Update Program ID

Copy the program ID and update it in:
- `lib.rs` (declare_id! macro)
- `Anchor.toml` (programs.devnet)

### 6. Deploy to Devnet

```bash
anchor deploy
```

## Program Structure

The program consists of the following main components:
- Flash loan pool initialization
- Deposit functionality
- Flash loan execution
- Repayment handling

## Usage

To execute a flash loan with this program, you'll need to specify:

1. Flash Loan Amount
   - Amount of SOL you want to borrow
   - Minimum amount: 0.1 SOL
   - Maximum amount: Based on pool liquidity

2. Token Pair Addresses
   - Token A: The first token you want to swap to (e.g., USDC)
   - Token B: The second token in the trading pair (e.g., BONK)

The flash loan process follows these steps:

1. Borrows SOL from the flash loan pool
2. Swaps SOL for Token A using Jupiter/Raydium
3. Swaps Token A for Token B
4. Executes arbitrage opportunity
5. Swaps Token B back to SOL
6. Repays the flash loan with fees
7. Transfers remaining profit to your wallet

Note: Ensure you have enough SOL in your wallet to cover transaction fees.

## Security Considerations

- Ensure all flash loans are repaid within the same transaction
- Verify token amounts and accounts carefully
- Follow security best practices for Solana program development


## License

MIT