//! General purpose ephermal_signers utility functions

use crate::state::proposal_versioned_transaction::VERSIONED_TRANSACTION_BUFFER_SEED;
use solana_program::pubkey::Pubkey;

/// Seed prefix for EphermalSigners PDAs
pub const EPHERMAL_SIGNER_SEED: &[u8] = b"ephemeral_signer";

/// Return a tuple of ephemeral_signer_keys and ephemeral_signer_seeds derived
/// from the given `ephemeral_signer_bumps` and `transaction_proposal`.
pub fn derive_ephemeral_signers(
    program_id: &Pubkey,
    transaction_proposal: &Pubkey,
    ephemeral_signer_bumps: &[u8],
    transaction_index: u16,
) -> (Vec<Pubkey>, Vec<Vec<Vec<u8>>>) {
    ephemeral_signer_bumps
        .iter()
        .enumerate()
        .map(|(index, bump)| {
            let seeds = vec![
                VERSIONED_TRANSACTION_BUFFER_SEED.to_vec(),
                transaction_proposal.to_bytes().to_vec(),
                EPHERMAL_SIGNER_SEED.to_vec(),
                u16::try_from(transaction_index)
                    .unwrap()
                    .to_le_bytes()
                    .to_vec(),
                u8::try_from(index).unwrap().to_le_bytes().to_vec(),
                vec![*bump],
            ];

            (
                Pubkey::create_program_address(
                    seeds
                        .iter()
                        .map(Vec::as_slice)
                        .collect::<Vec<&[u8]>>()
                        .as_slice(),
                    program_id,
                )
                .unwrap(),
                seeds,
            )
        })
        .unzip()
}
