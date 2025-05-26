//! Program state processor

use {
    crate::{
        error::GovernanceError,
        state::{
            governance::get_governance_data,
            proposal::get_proposal_data_for_governance,
            proposal_transaction_buffer::{
                get_proposal_transaction_buffer_address,
                get_proposal_transaction_buffer_data_for_proposal,
            },
            token_owner_record::get_token_owner_record_data_for_proposal_owner,
        },
    },
    solana_program::{
        account_info::{next_account_info, AccountInfo},
        clock::Clock,
        entrypoint::ProgramResult,
        msg,
        pubkey::Pubkey,
        sysvar::Sysvar,
    },
    spl_governance_tools::account::dispose_account,
};

/// Processes CloseTransactionBuffer instruction
pub fn process_close_transaction_buffer(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    // Index of the buffer account to seed the account derivation
    buffer_index: u8,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let governance_info = next_account_info(account_info_iter)?; // 0
    let proposal_info = next_account_info(account_info_iter)?; // 1
    let token_owner_record_info = next_account_info(account_info_iter)?; // 2
    let governance_authority_info = next_account_info(account_info_iter)?; // 3

    let proposal_transaction_buffer_info = next_account_info(account_info_iter)?; // 4

    let beneficiary_info = next_account_info(account_info_iter)?; // 5

    if !beneficiary_info.is_signer {
        return Err(GovernanceError::TransactionCreatorMustSign.into());
    }

    if proposal_transaction_buffer_info.data_is_empty() {
        return Err(GovernanceError::TransactionBufferAlreadyExists.into());
    }

    let clock = Clock::get()?;

    // Governance account is no longer used and it's deserialized only to validate
    // the provided account
    let governance_data = get_governance_data(program_id, governance_info)?;

    let proposal_data =
        get_proposal_data_for_governance(program_id, proposal_info, governance_info.key)?;

    // Check if the proposal is in the draft stage.
    // even if the transaction buffer has not been inserted into the final transaction proposal
    proposal_data.assert_can_cancel(&governance_data.config, clock.unix_timestamp)?;

    let token_owner_record_data = get_token_owner_record_data_for_proposal_owner(
        program_id,
        token_owner_record_info,
        &proposal_data.token_owner_record,
    )?;

    token_owner_record_data.assert_token_owner_or_delegate_is_signer(governance_authority_info)?;
    let _proposal_transaction_buffer_data = get_proposal_transaction_buffer_data_for_proposal(
        program_id,
        proposal_transaction_buffer_info,
        proposal_info.key,
    )?;

    let proposal_transaction_buffer_address = get_proposal_transaction_buffer_address(
        program_id,
        proposal_info.key,
        &beneficiary_info.key,
        &buffer_index.to_le_bytes(),
    );

    if proposal_transaction_buffer_address != *proposal_transaction_buffer_info.key {
        msg!("Proposal transaction buffer address does not match");
        return Err(GovernanceError::InvalidAccountFound.into());
    }

    dispose_account(proposal_transaction_buffer_info, beneficiary_info)?;

    Ok(())
}
