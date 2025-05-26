//! Program state processor

use {
    crate::{
        error::GovernanceError,
        state::{
            enums::{GovernanceAccountType, TransactionExecutionStatus},
            governance::get_governance_data,
            proposal::get_proposal_data_for_governance,
            proposal_versioned_transaction::{
                get_proposal_versioned_transaction_address_seeds, ProposalVersionedTransaction,
                VERSIONED_TRANSACTION_BUFFER_SEED,
            },
            token_owner_record::get_token_owner_record_data_for_proposal_owner,
        },
        tools::{ephermal_signers::EPHERMAL_SIGNER_SEED, transaction_message::TransactionMessage},
    },
    borsh::BorshDeserialize,
    solana_program::{
        account_info::{next_account_info, AccountInfo},
        entrypoint::ProgramResult,
        pubkey::Pubkey,
        rent::Rent,
        sysvar::Sysvar,
    },
    spl_governance_tools::account::create_and_serialize_account_signed,
    std::cmp::Ordering,
};

/// Processes InsertVersionedTransaction instruction
pub fn process_insert_versioned_transaction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    option_index: u8,
    // Number of ephemeral signing PDAs required by the transaction.
    ephemeral_signers: u8,
    transaction_index: u16,
    transaction_message: Vec<u8>,
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();

    let governance_info = next_account_info(account_info_iter)?; // 0
    let proposal_info = next_account_info(account_info_iter)?; // 1
    let token_owner_record_info = next_account_info(account_info_iter)?; // 2
    let governance_authority_info = next_account_info(account_info_iter)?; // 3

    let proposal_versioned_transaction_info = next_account_info(account_info_iter)?; // 4

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
        transaction_message,
        proposal_info,
        proposal_versioned_transaction_info,
        payer_info,
        system_info,
        rent,
    )?;

    Ok(())
}

// Generic function to create versioned_transaction account
pub fn process_create_versioned_transaction_account<'a>(
    program_id: &Pubkey,
    option_index: u8,
    ephemeral_signers: u8,
    transaction_index: u16,
    transaction_message: Vec<u8>,
    proposal_info: &AccountInfo<'a>,
    proposal_versioned_transaction_info: &AccountInfo<'a>,
    payer_info: &AccountInfo<'a>,
    system_info: &AccountInfo<'a>,
    rent: &Rent,
) -> ProgramResult {
    let transaction_message = TransactionMessage::deserialize(&mut transaction_message.as_slice())?;

    let ephemeral_signer_bumps: Vec<u8> = (0..ephemeral_signers)
        .map(|ephemeral_signer_index| {
            let ephemeral_signer_seeds = &[
                VERSIONED_TRANSACTION_BUFFER_SEED,
                proposal_versioned_transaction_info.key.as_ref(),
                EPHERMAL_SIGNER_SEED,
                &transaction_index.to_le_bytes(),
                &ephemeral_signer_index.to_le_bytes(),
            ];

            let (_, bump) = Pubkey::find_program_address(ephemeral_signer_seeds, program_id);
            bump
        })
        .collect();

    let proposal_versioned_transaction_data = ProposalVersionedTransaction {
        account_type: GovernanceAccountType::ProposalVersionedTransaction,
        proposal: *proposal_info.key,
        option_index,
        transaction_index,
        executed_at: None,
        execution_index: 0,
        ephemeral_signer_bumps,
        message: transaction_message.try_into()?,
        execution_status: TransactionExecutionStatus::None,
    };
    create_and_serialize_account_signed::<ProposalVersionedTransaction>(
        payer_info,
        proposal_versioned_transaction_info,
        &proposal_versioned_transaction_data,
        &get_proposal_versioned_transaction_address_seeds(
            proposal_info.key,
            &option_index.to_le_bytes(),
            &transaction_index.to_le_bytes(),
        ),
        program_id,
        system_info,
        rent,
        0,
    )?;
    Ok(())
}
