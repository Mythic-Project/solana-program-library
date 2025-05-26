//! Program state processor

use {
    super::process_create_versioned_transaction_account,
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
        entrypoint::ProgramResult,
        msg,
        pubkey::Pubkey,
        rent::Rent,
        sysvar::Sysvar,
    },
    spl_governance_tools::account::dispose_account,
    std::cmp::Ordering,
};

/// Processes InsertVersionedTransactionFromBuffer instruction
pub fn process_insert_versioned_transaction_from_buffer(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    option_index: u8,
    // Number of ephemeral signing PDAs required by the transaction.
    ephemeral_signers: u8,
    transaction_index: u16,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let governance_info = next_account_info(account_info_iter)?; // 0
    let proposal_info = next_account_info(account_info_iter)?; // 1
    let token_owner_record_info = next_account_info(account_info_iter)?; // 2
    let governance_authority_info = next_account_info(account_info_iter)?; // 3

    let proposal_versioned_transaction_info = next_account_info(account_info_iter)?; // 4
    let proposal_transaction_buffer_info = next_account_info(account_info_iter)?; // 4

    let payer_info = next_account_info(account_info_iter)?; // 5
    let system_info = next_account_info(account_info_iter)?; // 6
    let rent = &Rent::get()?;

    if !payer_info.is_signer {
        return Err(GovernanceError::TransactionCreatorMustSign.into());
    }
    if !proposal_versioned_transaction_info.data_is_empty() {
        return Err(GovernanceError::VersionedTransactionAlreadyExists.into());
    }

    // Governance account is no longer used and it's deserialized only to validate
    // the provided account
    let _governance_data = get_governance_data(program_id, governance_info)?;

    let mut proposal_data =
        get_proposal_data_for_governance(program_id, proposal_info, governance_info.key)?;
    proposal_data.assert_can_edit_instructions()?;

    let token_owner_record_data = get_token_owner_record_data_for_proposal_owner(
        program_id,
        token_owner_record_info,
        &proposal_data.token_owner_record,
    )?;

    token_owner_record_data.assert_token_owner_or_delegate_is_signer(governance_authority_info)?;

    let proposal_transaction_buffer_data = get_proposal_transaction_buffer_data_for_proposal(
        program_id,
        proposal_transaction_buffer_info,
        proposal_info.key,
    )?;

    let proposal_transaction_buffer_address = get_proposal_transaction_buffer_address(
        program_id,
        proposal_info.key,
        payer_info.key,
        &proposal_transaction_buffer_data.buffer_index.to_le_bytes(),
    );
    if proposal_transaction_buffer_address != *proposal_transaction_buffer_info.key {
        msg!("Proposal transaction buffer address does not match");
        return Err(GovernanceError::InvalidAccountFound.into());
    }
    proposal_transaction_buffer_data.validate_hash()?;
    proposal_transaction_buffer_data.validate_size()?;

    let option = &mut proposal_data.options[option_index as usize];

    match transaction_index.cmp(&option.transactions_next_index) {
        Ordering::Greater => return Err(GovernanceError::InvalidTransactionIndex.into()),
        // If the index is the same as transactions_next_index then we are adding a new transaction
        // If the index is below transactions_next_index then we are inserting into an existing
        // empty space
        Ordering::Equal => {
            option.transactions_next_index = option.transactions_next_index.checked_add(1).unwrap();
        }
        Ordering::Less => {}
    }

    option.transactions_count = option.transactions_count.checked_add(1).unwrap();
    proposal_data.serialize(&mut proposal_info.data.borrow_mut()[..])?;

    process_create_versioned_transaction_account(
        program_id,
        option_index,
        ephemeral_signers,
        transaction_index,
        proposal_transaction_buffer_data.buffer,
        proposal_info,
        proposal_versioned_transaction_info,
        payer_info,
        system_info,
        rent,
    )?;

    dispose_account(proposal_transaction_buffer_info, payer_info)?;

    Ok(())
}
