use anchor_lang::prelude::*;
use anchor_spl::{associated_token::AssociatedToken, token::{CloseAccount, Mint, Token, TokenAccount, Transfer}};

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");



#[error_code]
pub enum ErrorCode {
    #[msg("Wallet to withdraw from is not owned by owner")]
    WalletToWithdrawFromInvalid,
    #[msg("State index is inconsistent")]
    InvalidStateIdx,
    #[msg("Delegate is not set correctly")]
    DelegateNotSetCorrectly,
    #[msg("Stage is invalid")]
    StageInvalid,
}

fn transfer_escrow_out<'info>(
    user_sending: AccountInfo<'info>,
    user_receiving: AccountInfo<'info>,
    mint_of_token_being_sent: AccountInfo<'info>,
    escrow_wallet: &mut Account<'info, TokenAccount>,
    application_idx: u64,
    state: AccountInfo<'info>,
    state_bump: u8,
    token_program: AccountInfo<'info>,
    destination_wallet: AccountInfo<'info>,
    amount: u64
) -> Result<()> {

    // Sign on behalf of our PDA.
    let bump_vector = state_bump.to_le_bytes();
    let mint_of_token_being_sent_pk = mint_of_token_being_sent.key().clone();
    let application_idx_bytes = application_idx.to_le_bytes();
    let inner = vec![
        b"state".as_ref(),
        user_sending.key.as_ref(),
        user_receiving.key.as_ref(),
        mint_of_token_being_sent_pk.as_ref(), 
        application_idx_bytes.as_ref(),
        bump_vector.as_ref(),
    ];
    let outer = vec![inner.as_slice()];

    // Perform the actual transfer
    let transfer_instruction = Transfer{
        from: escrow_wallet.to_account_info(),
        to: destination_wallet,
        authority: state.to_account_info(),
    };
    let cpi_ctx = CpiContext::new_with_signer(
        token_program.to_account_info(),
        transfer_instruction,
        outer.as_slice(),
    );
    anchor_spl::token::transfer(cpi_ctx, amount)?;


    // Use the `reload()` function on an account to reload it's state. Since we performed the
    // transfer, we are expecting the `amount` field to have changed.
    let should_close = {
        escrow_wallet.reload()?;
        escrow_wallet.amount == 0
    };

    // If token account has no more tokens, it should be wiped out since it has no other use case.
    if should_close {
        let ca = CloseAccount{
            account: escrow_wallet.to_account_info(),
            destination: user_sending.to_account_info(),
            authority: state.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            token_program.to_account_info(),
            ca,
            outer.as_slice(),
        );
        anchor_spl::token::close_account(cpi_ctx)?;
    }

    Ok(())
}


#[program]
pub mod sample_on_solana {
    use super::*;
    use anchor_spl::token::Transfer;

    pub fn complete_grant(ctx: Context<CompleteGrant>, application_idx: u64, state_bump: u8, _wallet_bump: u8) -> Result<()> {
        if Stage::from(ctx.accounts.application_state.stage)? != Stage::FundsDeposited {
            msg!("Stage is invalid, state stage is {}", ctx.accounts.application_state.stage);
            return Err(ErrorCode::StageInvalid.into());
        }

        transfer_escrow_out(
            ctx.accounts.user_sending.to_account_info(),
            ctx.accounts.user_receiving.to_account_info(),
            ctx.accounts.mint_of_token_being_sent.to_account_info(),
            &mut ctx.accounts.escrow_wallet_state,
            application_idx,
            ctx.accounts.application_state.to_account_info(),
            state_bump,
            ctx.accounts.token_program.to_account_info(),
            ctx.accounts.wallet_to_deposit_to.to_account_info(),
            ctx.accounts.application_state.amount_tokens
        )?;

        let state = &mut ctx.accounts.application_state;
        state.stage = Stage::EscrowComplete.to_code();
        Ok(())
    }

    pub fn pull_back(ctx: Context<PullBackInstruction>, application_idx: u64, state_bump: u8, _wallet_bump: u8) -> Result<()> {
        let current_stage = Stage::from(ctx.accounts.application_state.stage)?;
        let is_valid_stage = current_stage == Stage::FundsDeposited || current_stage == Stage::PullBackComplete;
        if !is_valid_stage {
            msg!("Stage is invalid, state stage is {}", ctx.accounts.application_state.stage);
            return Err(ErrorCode::StageInvalid.into());
        }

        let wallet_amount = ctx.accounts.escrow_wallet_state.amount;
        transfer_escrow_out(
            ctx.accounts.user_sending.to_account_info(),
            ctx.accounts.user_receiving.to_account_info(),
            ctx.accounts.mint_of_token_being_sent.to_account_info(),
            &mut ctx.accounts.escrow_wallet_state,
            application_idx,
            ctx.accounts.application_state.to_account_info(),
            state_bump,
            ctx.accounts.token_program.to_account_info(),
            ctx.accounts.refund_wallet.to_account_info(),
            wallet_amount,
        )?;
        let state = &mut ctx.accounts.application_state;
        state.stage = Stage::PullBackComplete.to_code();

        Ok(())
    }

    pub fn initialize_new_grant(ctx: Context<InitializeNewGrant>, application_idx: u64, state_bump: u8, _wallet_bump: u8, amount: u64) -> Result<()> {

        // Set the state attributes
        let state = &mut ctx.accounts.application_state;
        state.idx = application_idx;
        state.user_sending = ctx.accounts.user_sending.key().clone();
        state.user_receiving = ctx.accounts.user_receiving.key().clone();
        state.mint_of_token_being_sent = ctx.accounts.mint_of_token_being_sent.key().clone();
        state.escrow_wallet = ctx.accounts.escrow_wallet_state.key().clone();
        state.amount_tokens = amount;

        msg!("Initialized new Safe Transfer instance for {}", amount);

        let bump_vector = state_bump.to_le_bytes();
        let mint_of_token_being_sent_pk = ctx.accounts.mint_of_token_being_sent.key().clone();
        let application_idx_bytes = application_idx.to_le_bytes();
        let inner = vec![
            b"state".as_ref(),
            ctx.accounts.user_sending.key.as_ref(),
            ctx.accounts.user_receiving.key.as_ref(),
            mint_of_token_being_sent_pk.as_ref(), 
            application_idx_bytes.as_ref(),
            bump_vector.as_ref(),
        ];
        let outer = vec![inner.as_slice()];

        // Below is the actual instruction that we are going to send to the Token program.
        let transfer_instruction = Transfer{
            from: ctx.accounts.wallet_to_withdraw_from.to_account_info(),
            to: ctx.accounts.escrow_wallet_state.to_account_info(),
            authority: ctx.accounts.user_sending.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            transfer_instruction,
            outer.as_slice(),
        );

        // The `?` at the end will cause the function to return early in case of an error.
        // This pattern is common in Rust.
        anchor_spl::token::transfer(cpi_ctx, state.amount_tokens)?;

        // Mark stage as deposited.
        state.stage = Stage::FundsDeposited.to_code();
        Ok(())
    }


    // pub fn create(ctx: Context<Create>) -> Result<()> {
    //     let counter_account = &mut ctx.accounts.counter_account;
    //     counter_account.count = 0;
    //     Ok(())
    // }

    // pub fn increment(ctx: Context<Increment>) -> Result<()> {
    //     let counter_account = &mut ctx.accounts.counter_account;
    //     counter_account.count += 1;
    //     Ok(())
    // }
}

#[derive(Accounts)]
#[instruction(instance_bump: u8, wallet_bump: u8)]
pub struct Initialize<'info> {
    #[account(
        seeds=[b"instance".as_ref(), user.key.as_ref()],
        //bump = instance_bump,
        bump,
    )]
    /// CHECK: This is not dangerous because we don't read or write from this account
    instance: AccountInfo<'info>,
    #[account(
        init,
        payer = user,
        seeds=[b"wallet".as_ref(), user.key.as_ref(), mint.key().as_ref()],
        //bump = wallet_bump,
        bump,
        token::mint = mint,
        token::authority = instance,
    )]
    wallet: Account<'info, TokenAccount>,
    #[account(mut)]
    mint: Account<'info, Mint>,
    #[account(mut)]
    user: Signer<'info>,
    system_program: Program<'info, System>,
    token_program: Program<'info, Token>,
    rent: Sysvar<'info, Rent>,
}


// Each state corresponds with a separate transaction and represents different moments in the lifecycle
// of the app.
//
// FundsDeposited -> EscrowComplete
//                OR
//                -> PullBackComplete
//
#[derive(Clone, Copy, PartialEq)]
pub enum Stage {
    // Kal withdrew funds from Consumer and deposited them into the escrow wallet
    FundsDeposited,

    // {from FundsDeposited} Supplier withdrew the funds from the escrow. We are done.
    EscrowComplete,

    // {from FundsDeposited} Consumer pulled back the funds
    PullBackComplete,
}

impl Stage {
    fn to_code(&self) -> u8 {
        match self {
            Stage::FundsDeposited => 1,
            Stage::EscrowComplete => 2,
            Stage::PullBackComplete => 3,
        }
    }

    fn from(val: u8) -> anchor_lang::Result<Stage> {
        match val {
            1 => Ok(Stage::FundsDeposited),
            2 => Ok(Stage::EscrowComplete),
            3 => Ok(Stage::PullBackComplete),
            unknown_value => {
                msg!("Unknown stage: {}", unknown_value);
                Err(ErrorCode::StageInvalid.into())
            }
        }
    }
}

// 1 State account instance == 1 Kal instance
#[account]
#[derive(Default)]
pub struct State {

    // A primary key that allows us to derive other important accounts
    idx: u64,
    
    // Consumer
    user_sending: Pubkey,

    // Supplier
    user_receiving: Pubkey,

    // The Mint of the token that Consumer wants to send to Supplier
    mint_of_token_being_sent: Pubkey,

    // The escrow wallet
    escrow_wallet: Pubkey,

    // The amount of tokens Consumer wants to send to Supplier
    amount_tokens: u64,

    // An enumm that is to represent some kind of state machine
    stage: u8,
}

#[derive(Accounts)]
#[instruction(application_idx: u64, state_bump: u8, wallet_bump: u8)]
pub struct InitializeNewGrant<'info> {
    // Derived PDAs
    #[account(
        init,
        payer = user_sending,
        seeds=[b"state".as_ref(), user_sending.key().as_ref(), user_receiving.key.as_ref(), mint_of_token_being_sent.key().as_ref(), application_idx.to_le_bytes().as_ref()],
        bump,
        space = 10240,
        //bump,
    )]
    application_state: Account<'info, State>,
    #[account(
        init,
        payer = user_sending,
        seeds=[b"wallet".as_ref(), user_sending.key().as_ref(), user_receiving.key.as_ref(), mint_of_token_being_sent.key().as_ref(), application_idx.to_le_bytes().as_ref()],
        //bump = wallet_bump,
        bump,
        token::mint=mint_of_token_being_sent,
        token::authority=application_state,
    )]
    escrow_wallet_state: Account<'info, TokenAccount>,

    // Users and accounts in the system
    #[account(mut)]
    user_sending: Signer<'info>,                     // Consumer
    /// CHECK: This is not dangerous because we don't read or write from this account
    user_receiving: AccountInfo<'info>,              // Supplier
    mint_of_token_being_sent: Account<'info, Mint>,  // USDC

    // Consumer's USDC wallet that has already approved the escrow wallet
    #[account(
        mut,
        constraint=wallet_to_withdraw_from.owner == user_sending.key(),
        constraint=wallet_to_withdraw_from.mint == mint_of_token_being_sent.key()
    )]
    wallet_to_withdraw_from: Account<'info, TokenAccount>,

    // Application level accounts
    system_program: Program<'info, System>,
    token_program: Program<'info, Token>,
    rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
#[instruction(application_idx: u64, state_bump: u8, wallet_bump: u8)]
pub struct CompleteGrant<'info> {
    #[account(
        mut,
        seeds=[b"state".as_ref(), user_sending.key().as_ref(), user_receiving.key.as_ref(), mint_of_token_being_sent.key().as_ref(), application_idx.to_le_bytes().as_ref()],
        bump = state_bump,
        has_one = user_sending,
        has_one = user_receiving,
        has_one = mint_of_token_being_sent,
    )]
    application_state: Account<'info, State>,
    #[account(
        mut,
        seeds=[b"wallet".as_ref(), user_sending.key().as_ref(), user_receiving.key.as_ref(), mint_of_token_being_sent.key().as_ref(), application_idx.to_le_bytes().as_ref()],
        bump = wallet_bump,
    )]
    escrow_wallet_state: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = user_receiving,
        associated_token::mint = mint_of_token_being_sent,
        associated_token::authority = user_receiving,
    )]
    wallet_to_deposit_to: Account<'info, TokenAccount>,   // Supplier's USDC wallet (will be initialized if it did not exist)

    // Users and accounts in the system
    #[account(mut)]
    /// CHECK: This is not dangerous because we don't read or write from this account
    user_sending: AccountInfo<'info>,                     // Consumer
    #[account(mut)]
    user_receiving: Signer<'info>,                        // Supplier
    mint_of_token_being_sent: Account<'info, Mint>,       // USDC

    // Application level accounts
    system_program: Program<'info, System>,
    token_program: Program<'info, Token>,
    associated_token_program: Program<'info, AssociatedToken>,
    rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
#[instruction(application_idx: u64, state_bump: u8, wallet_bump: u8)]
pub struct PullBackInstruction<'info> {
    #[account(
        mut,
        seeds=[b"state".as_ref(), user_sending.key().as_ref(), user_receiving.key.as_ref(), mint_of_token_being_sent.key().as_ref(), application_idx.to_le_bytes().as_ref()],
        bump = state_bump,
        has_one = user_sending,
        has_one = user_receiving,
        has_one = mint_of_token_being_sent,
    )]
    application_state: Account<'info, State>,
    #[account(
        mut,
        seeds=[b"wallet".as_ref(), user_sending.key().as_ref(), user_receiving.key.as_ref(), mint_of_token_being_sent.key().as_ref(), application_idx.to_le_bytes().as_ref()],
        bump = wallet_bump,
    )]
    escrow_wallet_state: Account<'info, TokenAccount>,    
    // Users and accounts in the system
    #[account(mut)]
    user_sending: Signer<'info>,
    /// CHECK: This is not dangerous because we don't read or write from this account
    user_receiving: AccountInfo<'info>,
    mint_of_token_being_sent: Account<'info, Mint>,

    // Application level accounts
    system_program: Program<'info, System>,
    token_program: Program<'info, Token>,
    rent: Sysvar<'info, Rent>,

    // Wallet to deposit to
    #[account(
        mut,
        constraint=refund_wallet.owner == user_sending.key(),
        constraint=refund_wallet.mint == mint_of_token_being_sent.key()
    )]
    refund_wallet: Account<'info, TokenAccount>,
}
// #[derive(Accounts)]
// pub struct Create<'info> {
    
//     #[account(init, payer=user, space = 16+16)]
//     pub counter_account: Account<'info, CounterAccount>,
    
//     #[account(mut)]
//     pub user: Signer<'info>,
    
//     pub system_program: Program<'info, System>,
// }

// #[derive(Accounts)]
// pub struct Increment<'info> {
//     #[account(mut)]
//     pub counter_account: Account<'info, CounterAccount>,
// }


// #[account]
// pub struct CounterAccount {
//     pub count: u64,
// }


