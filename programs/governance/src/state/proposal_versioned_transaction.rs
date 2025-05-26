//! ProposalVersionedTransaction Account

use {
    crate::{
        error::GovernanceError,
        state::enums::{GovernanceAccountType, TransactionExecutionStatus},
        tools::transaction_message::{
            CompiledInstruction, MessageAddressTableLookup, TransactionMessage,
        },
    },
    borsh::{io::Write, BorshDeserialize, BorshSchema, BorshSerialize},
    solana_program::{
        account_info::AccountInfo, borsh1::get_instance_packed_len, clock::UnixTimestamp, msg,
        program_error::ProgramError, program_pack::IsInitialized, pubkey::Pubkey,
    },
    spl_governance_tools::account::{get_account_data, AccountMaxSize},
};

impl IsInitialized for ProposalVersionedTransaction {
    fn is_initialized(&self) -> bool {
        self.account_type == GovernanceAccountType::ProposalVersionedTransaction
    }
}

impl ProposalVersionedTransaction {
    /// Serializes account into the target buffer
    pub fn serialize<W: Write>(self, writer: W) -> Result<(), ProgramError> {
        borsh::to_writer(writer, &self)?;

        Ok(())
    }
}

/// Account for an instruction to be executed for Proposal
#[derive(Clone, Default, Debug, BorshDeserialize, BorshSerialize, BorshSchema)]
pub struct ProposalVersionedTransaction {
    /// Governance Account type
    pub account_type: GovernanceAccountType,

    /// The Proposal the instruction belongs to
    pub proposal: Pubkey,

    /// The option index the instruction belongs to
    pub option_index: u8,

    /// Unique transaction index within it's parent Proposal
    pub transaction_index: u16,

    /// Executed index must be sequential
    pub execution_index: u8,

    /// Executed at flag
    pub executed_at: Option<UnixTimestamp>,

    /// Instruction execution status
    pub execution_status: TransactionExecutionStatus,

    /// Derivation bumps for additional signers.
    /// Some transactions require multiple signers. Often these additional
    /// signers are "ephemeral" keypairs that are generated on the client
    /// with a sole purpose of signing the transaction and be discarded
    /// immediately after. When wrapping such transactions into proposals,
    /// we replace these "ephemeral" signing keypairs with PDAs derived from
    /// the ProposalVersionedTransaction's `transaction_index` and controlled by
    /// the Goverenance Program; during execution the program includes the
    /// seeds of these PDAs into the `invoke_signed` calls, thus "signing"
    /// on behalf of these PDAs.
    pub ephemeral_signer_bumps: Vec<u8>,

    /// data required for executing the transaction.
    pub message: ProposalTransactionMessage,
}

impl AccountMaxSize for ProposalVersionedTransaction {
    /// proposal versioned_transaction can only be created from
    /// proposal_transaction_message
    fn get_max_size(&self) -> Option<usize> {
        let message_size = get_instance_packed_len(&self.message).unwrap_or_default();

        Some(
            1 +   // account discriminator
            1 +   // account_type
            32 +  // proposal
            1 +   // option_index
            2 +   // transaction_index
            1 +   // execution_index
            9 +   // executed_at (Option<UnixTimestamp>)
            1 +   // execution_status
            4 + self.ephemeral_signer_bumps.len() +
            message_size +
            40, // additional overhead
        )
    }
}

impl ProposalVersionedTransaction {
    /// Reduces the ProposalVersionedTransaction to its default empty value and
    /// moves ownership of the data to the caller/return value.
    pub fn take(&mut self) -> ProposalVersionedTransaction {
        core::mem::take(self)
    }
}

/// ProposalTransactionMessage Account
#[derive(Clone, Debug, BorshDeserialize, BorshSerialize, Default, BorshSchema)]
pub struct ProposalTransactionMessage {
    /// The number of signer pubkeys in the account_keys vec.
    pub num_signers: u8,
    /// The number of writable signer pubkeys in the account_keys vec.
    pub num_writable_signers: u8,
    /// The number of writable non-signer pubkeys in the account_keys vec.
    pub num_writable_non_signers: u8,
    /// Unique account pubkeys (including program IDs) required for execution of
    /// the tx. The signer pubkeys appear at the beginning of the vec, with
    /// writable pubkeys first, and read-only pubkeys following.
    /// The non-signer pubkeys follow with writable pubkeys first and read-only
    /// ones following. Program IDs are also stored at the end of the vec
    /// along with other non-signer non-writable pubkeys:
    ///
    /// ```plaintext
    /// [pubkey1, pubkey2, pubkey3, pubkey4, pubkey5, pubkey6, pubkey7, pubkey8]
    ///  |---writable---|  |---readonly---|  |---writable---|  |---readonly---|
    ///  |------------signers-------------|  |----------non-signers-----------|
    /// ```
    pub account_keys: Vec<Pubkey>,
    /// List of instructions making up the tx.
    pub instructions: Vec<ProposalCompiledInstruction>,
    /// List of address table lookups used to load additional accounts
    /// for this transaction.
    pub address_table_lookups: Vec<VersionedTransactionMessageAddressTableLookup>,
}

impl ProposalTransactionMessage {
    /// Returns the number of all the account keys (static + dynamic) in the
    /// message.
    pub fn num_all_account_keys(&self) -> usize {
        let num_account_keys_from_lookups = self
            .address_table_lookups
            .iter()
            .map(|lookup| lookup.writable_indexes.len() + lookup.readonly_indexes.len())
            .sum::<usize>();

        self.account_keys.len() + num_account_keys_from_lookups
    }

    /// Returns true if the account at the specified index is a part of static
    /// `account_keys` and was requested to be writable.
    pub fn is_static_writable_index(&self, key_index: usize) -> bool {
        let num_account_keys = self.account_keys.len();
        let num_signers = usize::from(self.num_signers);
        let num_writable_signers = usize::from(self.num_writable_signers);
        let num_writable_non_signers = usize::from(self.num_writable_non_signers);

        if key_index >= num_account_keys {
            // `index` is not a part of static `account_keys`.
            return false;
        }

        if key_index < num_writable_signers {
            // `index` is within the range of writable signer keys.
            return true;
        }

        if key_index >= num_signers {
            // `index` is within the range of non-signer keys.
            let index_into_non_signers = key_index.saturating_sub(num_signers);
            // Whether `index` is within the range of writable non-signer keys.
            return index_into_non_signers < num_writable_non_signers;
        }

        false
    }

    /// Returns true if the account at the specified index was requested to be a
    /// signer.
    pub fn is_signer_index(&self, key_index: usize) -> bool {
        key_index < usize::from(self.num_signers)
    }
}

impl TryFrom<TransactionMessage> for ProposalTransactionMessage {
    type Error = ProgramError;

    fn try_from(message: TransactionMessage) -> Result<Self, ProgramError> {
        let account_keys: Vec<Pubkey> = message.account_keys.into();
        let instructions: Vec<CompiledInstruction> = message.instructions.into();
        let instructions: Vec<ProposalCompiledInstruction> = instructions
            .into_iter()
            .map(ProposalCompiledInstruction::from)
            .collect();
        let address_table_lookups: Vec<MessageAddressTableLookup> =
            message.address_table_lookups.into();

        let num_all_account_keys = account_keys.len()
            + address_table_lookups
                .iter()
                .map(|lookup| lookup.writable_indexes.len() + lookup.readonly_indexes.len())
                .sum::<usize>();

        if account_keys.len() <= usize::from(message.num_signers) {
            msg!("account_keys length is smaller than number of signers");
            return Err(GovernanceError::InvalidTransactionMessage.into());
        }

        if message.num_writable_signers > message.num_signers {
            msg!("message writable signers length is larger than number of signers, they must be equal or less");
            return Err(GovernanceError::InvalidTransactionMessage.into());
        }
        if usize::from(message.num_writable_non_signers)
            > account_keys
                .len()
                .saturating_sub(usize::from(message.num_signers))
        {
            return Err(GovernanceError::InvalidTransactionMessage.into());
        }

        // Validate that all program ID indices and account indices are within the
        // bounds of the account keys.
        for instruction in &instructions {
            if usize::from(instruction.program_id_index) >= num_all_account_keys {
                return Err(GovernanceError::InvalidTransactionMessage.into());
            }
            for account_index in &instruction.account_indexes {
                if usize::from(*account_index) >= num_all_account_keys {
                    return Err(GovernanceError::InvalidTransactionMessage.into());
                }
            }
        }

        Ok(Self {
            num_signers: message.num_signers,
            num_writable_signers: message.num_writable_signers,
            num_writable_non_signers: message.num_writable_non_signers,
            account_keys,
            instructions,
            address_table_lookups: address_table_lookups
                .into_iter()
                .map(VersionedTransactionMessageAddressTableLookup::from)
                .collect(),
        })
    }
}

/// Concise serialization schema for instructions that make up a transaction.
/// Closely mimics the Solana transaction wire format.
#[derive(Clone, Debug, BorshDeserialize, BorshSerialize, BorshSchema)]
pub struct ProposalCompiledInstruction {
    /// Indices of the program_id in tx's account_keys
    pub program_id_index: u8,
    /// Indices into the tx's `account_keys` list indicating which accounts to
    /// pass to the instruction.
    pub account_indexes: Vec<u8>,
    /// Instruction data.
    pub data: Vec<u8>,
}

impl From<CompiledInstruction> for ProposalCompiledInstruction {
    fn from(compiled_instruction: CompiledInstruction) -> Self {
        Self {
            program_id_index: compiled_instruction.program_id_index,
            account_indexes: compiled_instruction.account_indexes.into(),
            data: compiled_instruction.data.into(),
        }
    }
}

/// Address table lookups describe an on-chain address lookup table to use
/// for loading more readonly and writable accounts into a transaction.
#[derive(Clone, Debug, PartialEq, Eq, BorshDeserialize, BorshSerialize, BorshSchema)]
pub struct VersionedTransactionMessageAddressTableLookup {
    /// Address lookup table account key.
    pub account_key: Pubkey,
    /// List of indexes used to load writable accounts.
    pub writable_indexes: Vec<u8>,
    /// List of indexes used to load readonly accounts.
    pub readonly_indexes: Vec<u8>,
}

impl From<MessageAddressTableLookup> for VersionedTransactionMessageAddressTableLookup {
    fn from(m: MessageAddressTableLookup) -> Self {
        Self {
            account_key: m.account_key,
            writable_indexes: m.writable_indexes.into(),
            readonly_indexes: m.readonly_indexes.into(),
        }
    }
}

/// Seed prefix for ProposalTransactionBuffer PDAs
pub const VERSIONED_TRANSACTION_BUFFER_SEED: &[u8] = b"version_transaction";

/// Returns ProposalVersionedTransaction PDA seeds
pub fn get_proposal_versioned_transaction_address_seeds<'a>(
    proposal: &'a Pubkey,
    option_index: &'a [u8; 1],               // u8 le bytes
    transaction_index_le_bytes: &'a [u8; 2], // u16 le bytes
) -> [&'a [u8]; 4] {
    [
        VERSIONED_TRANSACTION_BUFFER_SEED,
        proposal.as_ref(),
        option_index,
        transaction_index_le_bytes,
    ]
}

/// Returns ProposalVersionedTransaction PDA address
pub fn get_proposal_versioned_transaction_address<'a>(
    program_id: &Pubkey,
    proposal: &'a Pubkey,
    option_index_le_bytes: &'a [u8; 1],      // u8 le bytes
    transaction_index_le_bytes: &'a [u8; 2], // u16 le bytes
) -> Pubkey {
    Pubkey::find_program_address(
        &get_proposal_versioned_transaction_address_seeds(
            proposal,
            option_index_le_bytes,
            transaction_index_le_bytes,
        ),
        program_id,
    )
    .0
}

/// Deserializes ProposalVersionedTransaction account and checks owner program
pub fn get_proposal_versioned_transaction_data(
    program_id: &Pubkey,
    proposal_transaction_info: &AccountInfo,
) -> Result<ProposalVersionedTransaction, ProgramError> {
    get_account_data::<ProposalVersionedTransaction>(program_id, proposal_transaction_info)
}

/// Deserializes and returns ProposalVersionedTransaction account and checks it
/// belongs to the given Proposal
pub fn get_proposal_versioned_transaction_data_for_proposal(
    program_id: &Pubkey,
    proposal_transaction_info: &AccountInfo,
    proposal: &Pubkey,
) -> Result<ProposalVersionedTransaction, ProgramError> {
    let proposal_transaction_data =
        get_proposal_versioned_transaction_data(program_id, proposal_transaction_info)?;

    if proposal_transaction_data.proposal != *proposal {
        msg!("Mismatch of proposal for proposal versioned transaction");
        return Err(GovernanceError::InvalidProposalForProposalTransaction.into());
    }

    Ok(proposal_transaction_data)
}
