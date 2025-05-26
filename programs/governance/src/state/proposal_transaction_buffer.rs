//! ProposalTransactionBuffer Account

use {
    super::enums::GovernanceAccountType,
    crate::error::GovernanceError,
    borsh::{io::Write, BorshDeserialize, BorshSchema, BorshSerialize},
    solana_program::{
        account_info::AccountInfo, hash::hashv, msg, program_error::ProgramError,
        program_pack::IsInitialized, pubkey::Pubkey,
    },
    spl_governance_tools::account::{get_account_data, AccountMaxSize},
};

/// Maximum PDA allocation size in an inner ix is 10240 bytes.
/// 10240 - account contents = 10032 bytes
pub const MAX_BUFFER_SIZE: usize = 10032;

/// One of onchain buffer that consumes buffers and transforms them into
/// Versioned Transactions This account will be closed once it gets transformed
/// into ProposalVersionedTransaction
#[derive(Clone, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, BorshSchema)]
pub struct ProposalTransactionBuffer {
    /// Governance Account type
    pub account_type: GovernanceAccountType,
    /// The Proposal the transaction buffer belongs to
    pub proposal: Pubkey,
    /// Member of the Goverenance who created the TransactionBuffer.
    pub creator: Pubkey,
    /// Index to seed address derivation
    pub buffer_index: u8,
    /// Hash of the final assembled transaction message.
    pub final_buffer_hash: [u8; 32],
    /// The size of the final assembled transaction message.
    pub final_buffer_size: u16,
    /// The buffer of the transaction message.
    pub buffer: Vec<u8>,
}

impl AccountMaxSize for ProposalTransactionBuffer {
    fn get_max_size(&self) -> Option<usize> {
        Some(
            1 +   // account discriminator
            1 +   // account type
            32 +  // proposal
            32 +  // creator
            1 +   // buffer_index
            32 +  // transaction_message_hash
            2 +  // final_buffer_size
            4 + // vec length bytes
            self.final_buffer_size as usize, // buffer
        )
    }
}

impl IsInitialized for ProposalTransactionBuffer {
    fn is_initialized(&self) -> bool {
        self.account_type == GovernanceAccountType::ProposalTransactionBuffer
    }
}

impl ProposalTransactionBuffer {
    /// Size of onchain transaction buffer
    pub fn size(final_message_buffer_size: u16) -> Result<usize, ProgramError> {
        // Make sure final size is not greater than MAX_BUFFER_SIZE bytes.
        if (final_message_buffer_size as usize) > MAX_BUFFER_SIZE {
            return Err(GovernanceError::FinalBufferSizeExceeded.into());
        }
        Ok(
            1 +   // account discriminator
            32 +  // proposal
            32 +  // creator
            1 +   // buffer_index
            32 +  // transaction_message_hash
            2 +  // final_buffer_size
            4 + // vec length bytes
            final_message_buffer_size as usize, // buffer
        )
    }

    /// validate the final buffer has of the transaction buffer
    pub fn validate_hash(&self) -> Result<(), ProgramError> {
        let message_buffer_hash = hashv(&[self.buffer.as_slice()]);
        if message_buffer_hash.to_bytes() != self.final_buffer_hash {
            return Err(GovernanceError::FinalBufferHashMismatch.into());
        }
        Ok(())
    }

    /// validate the size of buffer of the transaction buffer
    pub fn validate_size(&self) -> Result<(), ProgramError> {
        if self.buffer.len() != self.final_buffer_size as usize {
            return Err(GovernanceError::FinalBufferSizeMismatch.into());
        }
        Ok(())
    }

    /// Check to make validate the size of buffer of the transaction buffer
    pub fn invariant(&self) -> Result<(), ProgramError> {
        if self.final_buffer_size as usize > MAX_BUFFER_SIZE {
            msg!("Current final buffer size: {}", self.final_buffer_size);
            return Err(GovernanceError::FinalBufferSizeExceeded.into());
        }
        if self.buffer.len() > self.final_buffer_size as usize {
            msg!(
                "Current buffer size: {}, is larger than final buffer size: {}",
                self.buffer.len(),
                self.final_buffer_size
            );
            return Err(GovernanceError::FinalBufferSizeExceeded.into());
        }

        Ok(())
    }

    /// Serializes account into the target buffer
    pub fn serialize<W: Write>(self, writer: W) -> Result<(), ProgramError> {
        borsh::to_writer(writer, &self)?;
        Ok(())
    }
}

/// Seed prefix for ProposalTransactionBuffer PDAs
pub const TRANSACTION_BUFFER_SEED: &[u8] = b"transaction_buffer";

/// Returns ProposalTransactionBuffer PDA seeds
pub fn get_proposal_transaction_buffer_address_seeds<'a>(
    proposal: &'a Pubkey,
    creator: &'a Pubkey,
    buffer_index: &'a [u8; 1], // u8 le bytes
) -> [&'a [u8]; 4] {
    [
        TRANSACTION_BUFFER_SEED,
        proposal.as_ref(),
        creator.as_ref(),
        buffer_index,
    ]
}

/// Returns ProposalTransactionBuffer PDA address
pub fn get_proposal_transaction_buffer_address<'a>(
    program_id: &Pubkey,
    proposal: &'a Pubkey,
    creator: &'a Pubkey,
    buffer_index: &'a [u8; 1], // u8 le bytes
) -> Pubkey {
    Pubkey::find_program_address(
        &get_proposal_transaction_buffer_address_seeds(proposal, creator, buffer_index),
        program_id,
    )
    .0
}

/// Deserializes ProposalTransactionBuffer account and checks owner program
pub fn get_proposal_transaction_buffer_data(
    program_id: &Pubkey,
    proposal_transaction_buffer_info: &AccountInfo,
) -> Result<ProposalTransactionBuffer, ProgramError> {
    let proposal_transaction_buffer_data = get_account_data::<ProposalTransactionBuffer>(
        program_id,
        proposal_transaction_buffer_info,
    )?;

    if proposal_transaction_buffer_data.account_type
        != GovernanceAccountType::ProposalTransactionBuffer
    {
        msg!("Invalid proposal transaction buffer account type");
        return Err(GovernanceError::InvalidGovernanceForProposal.into());
    }
    Ok(proposal_transaction_buffer_data)
}

/// Deserializes ProposalTransactionBuffer and validates it belongs to the given
/// Governance
pub fn get_proposal_transaction_buffer_data_for_proposal(
    program_id: &Pubkey,
    proposal_transaction_buffer_info: &AccountInfo,
    proposal: &Pubkey,
) -> Result<ProposalTransactionBuffer, ProgramError> {
    let proposal_transaction_buffer_data =
        get_proposal_transaction_buffer_data(program_id, proposal_transaction_buffer_info)?;

    if proposal_transaction_buffer_data.proposal != *proposal {
        msg!("Mismatch of proposal for proposal transaction buffer");
        return Err(GovernanceError::InvalidGovernanceForProposal.into());
    }

    Ok(proposal_transaction_buffer_data)
}
