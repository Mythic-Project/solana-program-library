//! Program state processor

use {
    crate::{
        error::GovernanceError,
        state::{
            enums::GovernanceAccountType,
            governance::get_governance_data,
            proposal::get_proposal_data_for_governance,
            proposal_transaction_buffer::{
                get_proposal_transaction_buffer_address_seeds, ProposalTransactionBuffer,
            },
            token_owner_record::get_token_owner_record_data_for_proposal_owner,
        },
    },
    solana_program::{
        account_info::{next_account_info, AccountInfo},
        entrypoint::ProgramResult,
        pubkey::Pubkey,
        rent::Rent,
        sysvar::Sysvar,
    },
    spl_governance_tools::account::create_and_serialize_account_signed,
};

/// Processes CreateTransactionBuffer instruction
pub fn process_create_transaction_buffer(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    // Index of the buffer account to seed the account derivation
    buffer_index: u8,
    // Hash (sha256) of the final assembled transaction message.
    final_buffer_hash: [u8; 32],
    // Final size of the buffer.
    final_buffer_size: u16,
    // Initial slice of the buffer.
    buffer: Vec<u8>,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let governance_info = next_account_info(account_info_iter)?; // 0
    let proposal_info = next_account_info(account_info_iter)?; // 1
    let token_owner_record_info = next_account_info(account_info_iter)?; // 2
    let governance_authority_info = next_account_info(account_info_iter)?; // 3

    let proposal_transaction_buffer_info = next_account_info(account_info_iter)?; // 4

    let payer_info = next_account_info(account_info_iter)?; // 5
    let system_info = next_account_info(account_info_iter)?; // 6
    let rent = &Rent::get()?;

    if !payer_info.is_signer {
        return Err(GovernanceError::TransactionCreatorMustSign.into());
    }
    if !proposal_transaction_buffer_info.data_is_empty() {
        return Err(GovernanceError::TransactionBufferAlreadyExists.into());
    }

    let _governance_data = get_governance_data(program_id, governance_info)?;

    let proposal_data =
        get_proposal_data_for_governance(program_id, proposal_info, governance_info.key)?;
    proposal_data.assert_can_edit_instructions()?;

    let token_owner_record_data = get_token_owner_record_data_for_proposal_owner(
        program_id,
        token_owner_record_info,
        &proposal_data.token_owner_record,
    )?;

    token_owner_record_data.assert_token_owner_or_delegate_is_signer(governance_authority_info)?;

    let proposal_transaction_buffer_data = ProposalTransactionBuffer {
        account_type: GovernanceAccountType::ProposalTransactionBuffer,
        proposal: *proposal_info.key,
        creator: *payer_info.key,
        buffer_index,
        final_buffer_hash,
        final_buffer_size,
        buffer,
    };
    // Validate transaction buffer data sizes
    proposal_transaction_buffer_data.invariant()?;

    create_and_serialize_account_signed::<ProposalTransactionBuffer>(
        payer_info,
        proposal_transaction_buffer_info,
        &proposal_transaction_buffer_data,
        &get_proposal_transaction_buffer_address_seeds(
            proposal_info.key,
            payer_info.key,
            &buffer_index.to_le_bytes(),
        ),
        program_id,
        system_info,
        rent,
        0,
    )?;

    Ok(())
}
