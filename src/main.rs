use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use solana_client::rpc_client::RpcClient;
use solana_sdk::{
    commitment_config::CommitmentConfig,
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
    signature::{read_keypair_file, Signature, Signer},
    system_instruction,
    transaction::Transaction,
};
use std::str::FromStr;

const PROGRAM_ID: &str = "5dkhUQ8PtXMnyQLzmg1HquD7dypQv2xQqdw49Q8kEqf3";
const ESCROW_ACCOUNT_SIZE: usize = 106; // 32+32+32+8+1+1 = 106 bytes

#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create a new escrow offer
    CreateOffer {
        #[arg(short = 'b', long)]
        buyer_keypair: String,
        #[arg(short = 'e', long)]
        escrow_keypair: String,
        #[arg(short = 'r', long)]
        arbiter: String,
        #[arg(short = 'm', long)]
        amount: u64,
    },
    /// Join an existing offer as seller
    JoinOffer {
        #[arg(short = 's', long)]
        seller_keypair: String,
        #[arg(short = 'e', long)]
        escrow_account: String,
    },
    /// Fund the escrow contract
    Fund {
        #[arg(short = 'b', long)]
        buyer_keypair: String,
        #[arg(short = 'e', long)]
        escrow_account: String,
    },
    /// Confirm the transaction as buyer
    Confirm {
        #[arg(short = 's', long)]
        seller_keypair: String,
        #[arg(short = 'e', long)]
        escrow_account: String,
    },
    /// Confirm as arbiter
    ArbiterConfirm {
        #[arg(short = 'a', long)]
        arbiter_keypair: String,
        #[arg(short = 'e', long)]
        escrow_account: String,
        #[arg(short = 's', long)]
        seller: String,
    },
    /// Cancel as arbiter
    ArbiterCancel {
        #[arg(short = 'a', long)]
        arbiter_keypair: String,
        #[arg(short = 'e', long)]
        escrow_account: String,
        #[arg(short = 'b', long)]
        buyer: String,
    },
    /// Mutual cancel by buyer and seller
    MutualCancel {
        #[arg(short = 'b', long)]
        buyer_keypair: String,
        #[arg(short = 's', long)]
        seller_keypair: String,
        #[arg(short = 'e', long)]
        escrow_account: String,
    },
    /// Close escrow account
    Close {
        #[arg(short = 'c', long)]
        closer_keypair: String,
        #[arg(short = 'e', long)]
        escrow_account: String,
    },
    /// Get escrow information
    Info {
        #[arg(short = 'e', long)]
        escrow_account: String,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum EscrowState {
    Uninitialized,
    Created,
    Initialized,
    Funded,
    Completed,
    Cancelled,
}

fn simulate_and_send(
    client: &RpcClient,
    transaction: &Transaction,
) -> Result<Signature> {
    let simulation_result = client.simulate_transaction(transaction)?;
    
    if let Some(logs) = simulation_result.value.logs {
        println!("Transaction logs:");
        for log in logs {
            println!("  {}", log);
        }
    }
    
    if let Some(err) = simulation_result.value.err {
        return Err(anyhow!("Simulation error: {:?}", err));
    }

    let signature = client.send_and_confirm_transaction(transaction)?;
    Ok(signature)
}

fn main() -> Result<()> {
    let args = Cli::parse();
    let rpc_url = "https://solana-devnet.g.alchemy.com/v2/h1IAKlzdhlhF0Yo8w9ajfdTTzVsAddJ5".to_string();
    let client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

    match args.command {
        Command::CreateOffer {
            buyer_keypair,
            escrow_keypair,
            arbiter,
            amount,
        } => create_offer(
            &client,
            &buyer_keypair,
            &escrow_keypair,
            &arbiter,
            amount,
        ),
        Command::JoinOffer {
            seller_keypair,
            escrow_account,
        } => join_offer(&client, &seller_keypair, &escrow_account),
        Command::Fund {
            buyer_keypair,
            escrow_account,
        } => fund_escrow(&client, &buyer_keypair, &escrow_account),
        Command::Confirm {
            seller_keypair,
            escrow_account,
        } => confirm_escrow(&client, &seller_keypair, &escrow_account),
        Command::ArbiterConfirm {
            arbiter_keypair,
            escrow_account,
            seller,
        } => arbiter_confirm(&client, &arbiter_keypair, &escrow_account, &seller),
        Command::ArbiterCancel {
            arbiter_keypair,
            escrow_account,
            buyer,
        } => arbiter_cancel(&client, &arbiter_keypair, &escrow_account, &buyer),
        Command::MutualCancel {
            buyer_keypair,
            seller_keypair,
            escrow_account,
        } => mutual_cancel(&client, &buyer_keypair, &seller_keypair, &escrow_account),
        Command::Close {
            closer_keypair,
            escrow_account,
        } => close_escrow(&client, &closer_keypair, &escrow_account),
        Command::Info { escrow_account } => get_escrow_info(&client, &escrow_account),
    }
}

fn check_state(client: &RpcClient, escrow_account: &str) -> Result<EscrowState> {
    let escrow_pubkey = Pubkey::from_str(escrow_account)?;
    let account_data = client.get_account_data(&escrow_pubkey)?;
    
    if account_data.len() < 106 {
        return Err(anyhow!("Invalid account data length"));
    }
    
    let state_byte = account_data[104];
    match state_byte {
        0 => Ok(EscrowState::Uninitialized),
        1 => Ok(EscrowState::Created),
        2 => Ok(EscrowState::Initialized),
        3 => Ok(EscrowState::Funded),
        4 => Ok(EscrowState::Completed),
        5 => Ok(EscrowState::Cancelled),
        _ => Err(anyhow!("Invalid state byte: {}", state_byte)),
    }
}

fn create_offer(
    client: &RpcClient,
    buyer_keypair_path: &str,
    escrow_keypair_path: &str,
    arbiter: &str,
    amount: u64,
) -> Result<()> {
    let buyer_keypair = read_keypair_file(buyer_keypair_path)
        .map_err(|_| anyhow!("Failed to read buyer keypair"))?;
    let escrow_keypair = read_keypair_file(escrow_keypair_path)
        .map_err(|_| anyhow!("Failed to read escrow keypair"))?;

    let program_id = Pubkey::from_str(PROGRAM_ID)?;
    let arbiter_pubkey = Pubkey::from_str(arbiter)?;

    // Create escrow account
    let create_account_ix = system_instruction::create_account(
        &buyer_keypair.pubkey(),
        &escrow_keypair.pubkey(),
        client
            .get_minimum_balance_for_rent_exemption(ESCROW_ACCOUNT_SIZE)
            .map_err(|e| anyhow!("Rent exemption error: {}", e))?,
        ESCROW_ACCOUNT_SIZE as u64,
        &program_id,
    );

    // Derive vault PDA
    let vault_pda = get_vault_pda(&escrow_keypair.pubkey(), &program_id);

    // Create offer instruction
    let data = {
        let mut data = vec![0]; // create_offer instruction index
        data.extend_from_slice(&amount.to_le_bytes()); // Amount (8 bytes)
        data.extend_from_slice(arbiter_pubkey.as_ref()); // Arbiter (32 bytes)
        data
    };

    let initialize_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(buyer_keypair.pubkey(), true),
            AccountMeta::new(escrow_keypair.pubkey(), false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
        ],
        data,
    };

    let blockhash = client
        .get_latest_blockhash()
        .map_err(|e| anyhow!("Blockhash error: {}", e))?;
    let message = Message::new(
        &[create_account_ix, initialize_ix],
        Some(&buyer_keypair.pubkey()),
    );
    let transaction = Transaction::new(
        &[&buyer_keypair, &escrow_keypair],
        message,
        blockhash,
    );

    let signature = simulate_and_send(client, &transaction)?;
    println!("Offer created successfully! Signature: {}", signature);
    Ok(())
}

fn join_offer(
    client: &RpcClient,
    seller_keypair_path: &str,
    escrow_account: &str,
) -> Result<()> {
    let seller_keypair = read_keypair_file(seller_keypair_path)
        .map_err(|_| anyhow!("Failed to read seller keypair"))?;
    let escrow_pubkey = Pubkey::from_str(escrow_account)?;
    let program_id = Pubkey::from_str(PROGRAM_ID)?;

    // Verify state
    match check_state(client, escrow_account)? {
        EscrowState::Created => {},
        other_state => return Err(anyhow!(
            "Escrow must be in Created state, current state: {:?}", 
            other_state
        )),
    }

    // Create join instruction
    let data = {
        let mut data = vec![1]; // join_offer instruction index
        data.extend_from_slice(seller_keypair.pubkey().as_ref()); // Seller pubkey
        data
    };

    let join_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(seller_keypair.pubkey(), true),
            AccountMeta::new(escrow_pubkey, false),
        ],
        data,
    };

    let blockhash = client
        .get_latest_blockhash()
        .map_err(|e| anyhow!("Blockhash error: {}", e))?;
    let message = Message::new(&[join_ix], Some(&seller_keypair.pubkey()));
    let transaction = Transaction::new(&[&seller_keypair], message, blockhash);

    let signature = simulate_and_send(client, &transaction)?;
    println!("Joined offer successfully! Signature: {}", signature);
    Ok(())
}

fn fund_escrow(
    client: &RpcClient,
    buyer_keypair_path: &str,
    escrow_account: &str,
) -> Result<()> {
    let buyer_keypair = read_keypair_file(buyer_keypair_path)
        .map_err(|_| anyhow!("Failed to read buyer keypair"))?;
    let escrow_pubkey = Pubkey::from_str(escrow_account)?;
    let program_id = Pubkey::from_str(PROGRAM_ID)?;

    // Verify state
    match check_state(client, escrow_account)? {
        EscrowState::Initialized => {},
        other_state => return Err(anyhow!(
            "Escrow must be in Initialized state, current state: {:?}", 
            other_state
        )),
    }

    let vault_pda = get_vault_pda(&escrow_pubkey, &program_id);

    let fund_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(buyer_keypair.pubkey(), true),
            AccountMeta::new(escrow_pubkey, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
        ],
        data: vec![2], // fund_escrow instruction index
    };

    let blockhash = client
        .get_latest_blockhash()
        .map_err(|e| anyhow!("Blockhash error: {}", e))?;
    let message = Message::new(&[fund_ix], Some(&buyer_keypair.pubkey()));
    let transaction = Transaction::new(&[&buyer_keypair], message, blockhash);

    let signature = simulate_and_send(client, &transaction)?;
    println!("Escrow funded successfully! Signature: {}", signature);
    Ok(())
}

fn confirm_escrow(
    client: &RpcClient,
    seller_keypair_path: &str,
    escrow_account: &str,
) -> Result<()> {
    let seller_keypair = read_keypair_file(seller_keypair_path)
        .map_err(|_| anyhow!("Failed to read seller keypair"))?;
    let escrow_pubkey = Pubkey::from_str(escrow_account)?;
    let program_id = Pubkey::from_str(PROGRAM_ID)?;

    // Verify state
    match check_state(client, escrow_account)? {
        EscrowState::Funded => {},
        other_state => return Err(anyhow!(
            "Escrow must be in Funded state, current state: {:?}", 
            other_state
        )),
    }

    let vault_pda = get_vault_pda(&escrow_pubkey, &program_id);

    let confirm_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(seller_keypair.pubkey(), true),
            AccountMeta::new(escrow_pubkey, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
        ],
        data: vec![3], // confirm_escrow instruction index
    };

    let blockhash = client
        .get_latest_blockhash()
        .map_err(|e| anyhow!("Blockhash error: {}", e))?;
    let message = Message::new(&[confirm_ix], Some(&seller_keypair.pubkey()));
    let transaction = Transaction::new(&[&seller_keypair], message, blockhash);

    let signature = simulate_and_send(client, &transaction)?;
    println!("Transaction confirmed! Signature: {}", signature);
    Ok(())
}

fn arbiter_confirm(
    client: &RpcClient,
    arbiter_keypair_path: &str,
    escrow_account: &str,
    seller: &str,
) -> Result<()> {
    let arbiter_keypair = read_keypair_file(arbiter_keypair_path)
        .map_err(|_| anyhow!("Failed to read arbiter keypair"))?;
    let escrow_pubkey = Pubkey::from_str(escrow_account)?;
    let seller_pubkey = Pubkey::from_str(seller)?;
    let program_id = Pubkey::from_str(PROGRAM_ID)?;

    // Verify state
    match check_state(client, escrow_account)? {
        EscrowState::Funded => {},
        other_state => return Err(anyhow!(
            "Escrow must be in Funded state, current state: {:?}", 
            other_state
        )),
    }

    let vault_pda = get_vault_pda(&escrow_pubkey, &program_id);

    let confirm_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(arbiter_keypair.pubkey(), true),
            AccountMeta::new(escrow_pubkey, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(seller_pubkey, false),
        ],
        data: vec![4], // arbiter_confirm instruction index
    };

    let blockhash = client
        .get_latest_blockhash()
        .map_err(|e| anyhow!("Blockhash error: {}", e))?;
    let message = Message::new(&[confirm_ix], Some(&arbiter_keypair.pubkey()));
    let transaction = Transaction::new(&[&arbiter_keypair], message, blockhash);

    let signature = simulate_and_send(client, &transaction)?;
    println!("Arbiter confirmed! Signature: {}", signature);
    Ok(())
}

fn arbiter_cancel(
    client: &RpcClient,
    arbiter_keypair_path: &str,
    escrow_account: &str,
    buyer: &str,
) -> Result<()> {
    let arbiter_keypair = read_keypair_file(arbiter_keypair_path)
        .map_err(|_| anyhow!("Failed to read arbiter keypair"))?;
    let escrow_pubkey = Pubkey::from_str(escrow_account)?;
    let buyer_pubkey = Pubkey::from_str(buyer)?;
    let program_id = Pubkey::from_str(PROGRAM_ID)?;

    // Verify state
    match check_state(client, escrow_account)? {
        EscrowState::Funded => {},
        other_state => return Err(anyhow!(
            "Escrow must be in Funded state, current state: {:?}", 
            other_state
        )),
    }

    let vault_pda = get_vault_pda(&escrow_pubkey, &program_id);

    let cancel_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(arbiter_keypair.pubkey(), true),
            AccountMeta::new(escrow_pubkey, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new(buyer_pubkey, false),
        ],
        data: vec![5], // arbiter_cancel instruction index
    };

    let blockhash = client
        .get_latest_blockhash()
        .map_err(|e| anyhow!("Blockhash error: {}", e))?;
    let message = Message::new(&[cancel_ix], Some(&arbiter_keypair.pubkey()));
    let transaction = Transaction::new(&[&arbiter_keypair], message, blockhash);

    let signature = simulate_and_send(client, &transaction)?;
    println!("Arbiter canceled! Signature: {}", signature);
    Ok(())
}

fn mutual_cancel(
    client: &RpcClient,
    buyer_keypair_path: &str,
    seller_keypair_path: &str,
    escrow_account: &str,
) -> Result<()> {
    let buyer_keypair = read_keypair_file(buyer_keypair_path)
        .map_err(|_| anyhow!("Failed to read buyer keypair"))?;
    let seller_keypair = read_keypair_file(seller_keypair_path)
        .map_err(|_| anyhow!("Failed to read seller keypair"))?;
    let escrow_pubkey = Pubkey::from_str(escrow_account)?;
    let program_id = Pubkey::from_str(PROGRAM_ID)?;

    // Verify state
    match check_state(client, escrow_account)? {
        EscrowState::Initialized | EscrowState::Funded => {},
        other_state => return Err(anyhow!(
            "Escrow must be in Initialized or Funded state, current state: {:?}", 
            other_state
        )),
    }

    let vault_pda = get_vault_pda(&escrow_pubkey, &program_id);

    let cancel_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(buyer_keypair.pubkey(), true),
            AccountMeta::new(seller_keypair.pubkey(), true),
            AccountMeta::new(escrow_pubkey, false),
            AccountMeta::new(vault_pda, false),
            AccountMeta::new_readonly(solana_sdk::system_program::id(), false),
        ],
        data: vec![8], // mutual_cancel instruction index
    };

    let blockhash = client.get_latest_blockhash()?;
    let message = Message::new(
        &[cancel_ix],
        Some(&buyer_keypair.pubkey()),
    );
    let transaction = Transaction::new(
        &[&buyer_keypair, &seller_keypair],
        message,
        blockhash,
    );

    let signature = simulate_and_send(client, &transaction)?;
    println!("Mutual cancel successful! Signature: {}", signature);
    Ok(())
}

fn close_escrow(
    client: &RpcClient,
    closer_keypair_path: &str,
    escrow_account: &str,
) -> Result<()> {
    let closer_keypair = read_keypair_file(closer_keypair_path)
        .map_err(|_| anyhow!("Failed to read closer keypair"))?;
    let escrow_pubkey = Pubkey::from_str(escrow_account)?;
    let program_id = Pubkey::from_str(PROGRAM_ID)?;

    // Verify state
    match check_state(client, escrow_account)? {
        EscrowState::Completed | EscrowState::Cancelled => {},
        other_state => return Err(anyhow!(
            "Escrow must be Completed or Cancelled, current state: {:?}", 
            other_state
        )),
    }

    let close_ix = Instruction {
        program_id,
        accounts: vec![
            AccountMeta::new(closer_keypair.pubkey(), true),
            AccountMeta::new(escrow_pubkey, false),
        ],
        data: vec![6], // close_escrow instruction index
    };

    let blockhash = client.get_latest_blockhash()?;
    let message = Message::new(&[close_ix], Some(&closer_keypair.pubkey()));
    let transaction = Transaction::new(&[&closer_keypair], message, blockhash);

    let signature = simulate_and_send(client, &transaction)?;
    println!("Escrow closed! Signature: {}", signature);
    Ok(())
}

fn get_escrow_info(
    client: &RpcClient,
    escrow_account: &str,
) -> Result<()> {
    let escrow_pubkey = Pubkey::from_str(escrow_account)?;
    let account_data = client.get_account_data(&escrow_pubkey)?;

    if account_data.len() < 106 {
        return Err(anyhow!("Invalid account data length"));
    }

    let buyer = Pubkey::new(&account_data[0..32]);
    let seller = Pubkey::new(&account_data[32..64]);
    let arbiter = Pubkey::new(&account_data[64..96]);
    let amount = u64::from_le_bytes(account_data[96..104].try_into()?);
    let state_byte = account_data[104];
    let vault_bump = account_data[105];

    let state = match state_byte {
        0 => "Uninitialized",
        1 => "Created",
        2 => "Initialized",
        3 => "Funded",
        4 => "Completed",
        5 => "Cancelled",
        _ => "Unknown",
    };

    println!("Escrow Information:");
    println!("====================");
    println!("State: {}", state);
    println!("Amount: {} lamports", amount);
    println!("Buyer: {}", buyer);
    println!("Seller: {}", seller);
    println!("Arbiter: {}", arbiter);
    println!("Vault Bump: {}", vault_bump);
    println!("====================");

    Ok(())
}

fn get_vault_pda(escrow_account: &Pubkey, program_id: &Pubkey) -> Pubkey {
    let (pda, _) = Pubkey::find_program_address(
        &[b"vault", escrow_account.as_ref()],
        program_id,
    );
    pda
}