//! Program state processor

use {
    crate::{
        error::GovernanceError,
        state::{
            enums::{ProposalState, TransactionExecutionStatus},
            governance::get_governance_data,
            native_treasury::get_native_treasury_address_seeds,
            proposal::{get_proposal_data_for_governance, OptionVoteResult},
            proposal_versioned_transaction::get_proposal_versioned_transaction_data_for_proposal,
        },
        tools::{
            ephermal_signers::derive_ephemeral_signers,
            executable_transaction_message::ExecutableTransactionMessage,
        },
    },
    solana_program::{
        account_info::{next_account_info, AccountInfo},
        clock::Clock,
        entrypoint::ProgramResult,
        pubkey::Pubkey,
        sysvar::Sysvar,
    },
};

/// Processes ExecuteVersionedTransaction instruction
pub fn process_execute_versioned_transaction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let governance_info = next_account_info(account_info_iter)?; // 0
    let proposal_info = next_account_info(account_info_iter)?; // 1
    let proposal_versioned_transaction_info = next_account_info(account_info_iter)?; // 2

    let clock = Clock::get()?;

    let governance_data = get_governance_data(program_id, governance_info)?;

    let mut proposal_data =
        get_proposal_data_for_governance(program_id, proposal_info, governance_info.key)?;

    let mut proposal_versioned_transaction_data =
        get_proposal_versioned_transaction_data_for_proposal(
            program_id,
            proposal_versioned_transaction_info,
            proposal_info.key,
        )?;

    proposal_data.assert_can_execute_versioned_transaction(
        &proposal_versioned_transaction_data,
        &governance_data.config,
        clock.unix_timestamp,
    )?;

    let transaction = proposal_versioned_transaction_data.take();

    // `remaining_accounts` must include the following accounts in the exact order:
    // 1. AddressLookupTable accounts in the order they appear in
    //    `message.address_table_lookups`.
    // 2. Accounts in the order they appear in `message.account_keys`.
    // 3. Accounts in the order they appear in `message.address_table_lookups`.
    let transaction_account_infos = account_info_iter.as_slice();

    let transaction_message = transaction.message;
    let num_lookups = transaction_message.address_table_lookups.len();

    let message_account_infos = transaction_account_infos
        .get(num_lookups..)
        .ok_or(GovernanceError::InvalidNumberOfAccounts)?;
    let address_lookup_table_account_infos = transaction_account_infos
        .get(..num_lookups)
        .ok_or(GovernanceError::InvalidNumberOfAccounts)?;

    // Sign the transaction using the governance PDA
    let mut governance_seeds = governance_data.get_governance_address_seeds()?.to_vec();
    let (governance_pubkey, bump_seed) =
        Pubkey::find_program_address(&governance_seeds, program_id);
    let bump = &[bump_seed];
    // It will not be included if it is not used in execute_message()
    governance_seeds.push(bump);

    // Sign the transaction using the governance treasury PDA
    let mut treasury_seeds = get_native_treasury_address_seeds(governance_info.key).to_vec();
    let (treasury_address, treasury_bump_seed) =
        Pubkey::find_program_address(&treasury_seeds, program_id);
    let treasury_bump = &[treasury_bump_seed];
    // It will not be included if it is not used in execute_message()
    treasury_seeds.push(treasury_bump);

    let (ephemeral_signer_keys, ephemeral_signer_seeds) = derive_ephemeral_signers(
        program_id,
        &proposal_versioned_transaction_info.key,
        &transaction.ephemeral_signer_bumps,
        proposal_versioned_transaction_data.transaction_index,
    );

    let executable_message = ExecutableTransactionMessage::new_validated(
        transaction_message,
        message_account_infos,
        address_lookup_table_account_infos,
        &treasury_address,
        &governance_pubkey,
        &ephemeral_signer_keys,
        proposal_data.voting_at_slot.unwrap()
    )?;

    // Protected accounts that cannot be used in execute_message()
    let protected_accounts = &[*proposal_info.key];

    // Execute the transaction message instructions one-by-one
    executable_message.execute_message(
        &governance_seeds[..],
        &treasury_seeds[..],
        &governance_pubkey,
        &treasury_address,
        &ephemeral_signer_seeds,
        protected_accounts,
    )?;

    // Update proposal and instruction accounts
    if proposal_data.state == ProposalState::Succeeded {
        proposal_data.executing_at = Some(clock.unix_timestamp);
        proposal_data.state = ProposalState::Executing;
    }
    let option =
        &mut proposal_data.options[proposal_versioned_transaction_data.option_index as usize];
    option.transactions_executed_count = option.transactions_executed_count.checked_add(1).unwrap();

    // Checking for Executing and ExecutingWithErrors states because instruction can
    // still be executed after being flagged with error The check for
    // instructions_executed_count ensures Proposal can't be transitioned to
    // Completed state from ExecutingWithErrors
    if (proposal_data.state == ProposalState::Executing
        || proposal_data.state == ProposalState::ExecutingWithErrors)
        && proposal_data
            .options
            .iter()
            .filter(|o| o.vote_result == OptionVoteResult::Succeeded)
            .all(|o| o.transactions_executed_count == o.transactions_count)
    {
        proposal_data.closed_at = Some(clock.unix_timestamp);
        proposal_data.state = ProposalState::Completed;
    }

    proposal_data.serialize(&mut proposal_info.data.borrow_mut()[..])?;

    proposal_versioned_transaction_data.executed_at = Some(clock.unix_timestamp);
    proposal_versioned_transaction_data.execution_status = TransactionExecutionStatus::Success;
    proposal_versioned_transaction_data
        .serialize(&mut proposal_versioned_transaction_info.data.borrow_mut()[..])?;

    Ok(())
}
