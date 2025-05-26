#![allow(clippy::arithmetic_side_effects)]
use {
    crate::tools::map_transaction_error,
    bincode::deserialize,
    borsh::{BorshDeserialize, BorshSerialize},
    cookies::{TokenAccountCookie, WalletCookie},
    solana_program::{
        borsh1::try_from_slice_unchecked, clock::Clock, instruction::Instruction,
        program_error::ProgramError, program_pack::Pack, pubkey::Pubkey, rent::Rent,
        stake_history::Epoch, system_instruction, system_program, sysvar,
    },
    solana_program_test::{ProgramTest, ProgramTestContext},
    solana_sdk::{
        account::{Account, AccountSharedData, WritableAccount},
        instruction::AccountMeta,
        signature::Keypair,
        signer::Signer,
        transaction::Transaction,
    },
    spl_tlv_account_resolution::{
        account::ExtraAccountMeta, seeds::Seed, state::ExtraAccountMetaList,
    },
    spl_token_2022::instruction::{set_authority, AuthorityType},
    spl_token_2022::{extension::ExtensionType, state::Mint},
    spl_token_client::token::ExtensionInitializationParams,
    spl_transfer_hook_interface::{
        get_extra_account_metas_address,
        instruction::{initialize_extra_account_meta_list, update_extra_account_meta_list},
    },
    std::borrow::Borrow,
    token_2022::{test_transfer_fee_config_with_keypairs, TransferFeeConfigWithKeypairs},
    tools::clone_keypair,
};

pub mod addins;
pub mod cookies;
pub mod mpl_core_tools;
pub mod token_2022;
pub mod tools;
pub mod versioned_transaction;

/// Program's test bench which captures test context, rent and payer and common
/// utility functions
pub struct ProgramTestBench {
    pub context: ProgramTestContext,
    pub rent: Rent,
    pub payer: Keypair,
    pub next_id: u8,
}

impl ProgramTestBench {
    /// Create new bench given a ProgramTest instance populated with all of the
    /// desired programs.
    pub async fn start_new(program_test: ProgramTest) -> Self {
        let context = program_test.start_with_context().await;
        let rent = context.banks_client.get_rent().await.unwrap();

        let payer = clone_keypair(&context.payer);

        Self {
            context,
            rent,
            payer,
            next_id: 0,
        }
    }

    pub fn get_unique_name(&mut self, prefix: &str) -> String {
        self.next_id += 1;

        format!("{}.{}", prefix, self.next_id)
    }

    pub async fn process_transaction(
        &mut self,
        instructions: &[Instruction],
        signers: Option<&[&Keypair]>,
    ) -> Result<(), ProgramError> {
        let mut transaction = Transaction::new_with_payer(instructions, Some(&self.payer.pubkey()));

        let mut all_signers = vec![&self.payer];

        if let Some(signers) = signers {
            all_signers.extend_from_slice(signers);
        }

        let recent_blockhash = self
            .context
            .banks_client
            .get_latest_blockhash()
            .await
            .unwrap();

        transaction.sign(&all_signers, recent_blockhash);

        self.context
            .banks_client
            .process_transaction(transaction)
            .await
            .map_err(|e| map_transaction_error(e.into()))?;

        Ok(())
    }

    pub async fn with_wallet(&mut self) -> WalletCookie {
        let account_rent = self.rent.minimum_balance(0);
        let account_keypair = Keypair::new();

        let create_account_ix = system_instruction::create_account(
            &self.context.payer.pubkey(),
            &account_keypair.pubkey(),
            account_rent,
            0,
            &system_program::id(),
        );

        self.process_transaction(&[create_account_ix], Some(&[&account_keypair]))
            .await
            .unwrap();

        let account = Account {
            lamports: account_rent,
            data: vec![],
            owner: system_program::id(),
            executable: false,
            rent_epoch: 0,
        };

        WalletCookie {
            address: account_keypair.pubkey(),
            account,
        }
    }

    pub async fn create_mint(
        &mut self,
        mint_keypair: &Keypair,
        mint_authority: &Pubkey,
        freeze_authority: Option<&Pubkey>,
    ) {
        let mint_rent = self.rent.minimum_balance(spl_token::state::Mint::LEN);

        let instructions = [
            system_instruction::create_account(
                &self.context.payer.pubkey(),
                &mint_keypair.pubkey(),
                mint_rent,
                spl_token::state::Mint::LEN as u64,
                &spl_token::id(),
            ),
            spl_token::instruction::initialize_mint(
                &spl_token::id(),
                &mint_keypair.pubkey(),
                mint_authority,
                freeze_authority,
                0,
            )
            .unwrap(),
        ];

        self.process_transaction(&instructions, Some(&[mint_keypair]))
            .await
            .unwrap();
    }

    pub async fn create_mint_2022(
        &mut self,
        mint_keypair: &Keypair,
        mint_authority: &Pubkey,
        freeze_authority: Option<&Pubkey>,
    ) {
        let mint_rent = self.rent.minimum_balance(spl_token_2022::state::Mint::LEN);

        let instructions = [
            system_instruction::create_account(
                &self.context.payer.pubkey(),
                &mint_keypair.pubkey(),
                mint_rent,
                spl_token_2022::state::Mint::LEN as u64,
                &spl_token_2022::id(),
            ),
            spl_token_2022::instruction::initialize_mint(
                &spl_token_2022::id(),
                &mint_keypair.pubkey(),
                mint_authority,
                freeze_authority,
                0,
            )
            .unwrap(),
        ];

        self.process_transaction(&instructions, Some(&[mint_keypair]))
            .await
            .unwrap();
    }

    pub async fn create_mint_2022_with_extensions(
        &mut self,
        mint_keypair: &Keypair,
        mint_authority: &Pubkey,
        freeze_authority: Option<&Pubkey>,
    ) {
        let extension_initialization_params = vec![
            ExtensionInitializationParams::MintCloseAuthority {
                close_authority: Some(*mint_authority),
            },
            ExtensionInitializationParams::PermanentDelegate {
                delegate: *mint_authority,
            },
        ];
        let extension_types = extension_initialization_params
            .iter()
            .map(|e| e.extension())
            .collect::<Vec<_>>();
        let space = ExtensionType::try_calculate_account_len::<Mint>(&extension_types).unwrap();
        let mint_rent = self.rent.minimum_balance(space);

        let mut instructions = vec![system_instruction::create_account(
            &self.context.payer.pubkey(),
            &mint_keypair.pubkey(),
            mint_rent,
            space as u64,
            &spl_token_2022::id(),
        )];

        for params in extension_initialization_params {
            instructions.push(
                params
                    .instruction(&spl_token_2022::id(), &mint_keypair.pubkey())
                    .unwrap(),
            );
        }

        instructions.push(
            spl_token_2022::instruction::initialize_mint(
                &spl_token_2022::id(),
                &mint_keypair.pubkey(),
                mint_authority,
                freeze_authority,
                0,
            )
            .unwrap(),
        );

        self.process_transaction(&instructions, Some(&[mint_keypair]))
            .await
            .unwrap();
    }

    pub async fn create_mint_2022_transfer_fee(
        &mut self,
        mint_keypair: &Keypair,
        mint_authority: &Pubkey,
        freeze_authority: Option<&Pubkey>,
    ) {
        let TransferFeeConfigWithKeypairs {
            transfer_fee_config_authority,
            withdraw_withheld_authority,
            transfer_fee_config,
            ..
        } = test_transfer_fee_config_with_keypairs();
        let transfer_fee_basis_points = u16::from(
            transfer_fee_config
                .newer_transfer_fee
                .transfer_fee_basis_points,
        );
        let maximum_fee = u64::from(transfer_fee_config.newer_transfer_fee.maximum_fee);
        let extension_initialization_params =
            vec![ExtensionInitializationParams::TransferFeeConfig {
                transfer_fee_config_authority: transfer_fee_config_authority.pubkey().into(),
                withdraw_withheld_authority: withdraw_withheld_authority.pubkey().into(),
                transfer_fee_basis_points,
                maximum_fee,
            }];
        let extension_types = extension_initialization_params
            .iter()
            .map(|e| e.extension())
            .collect::<Vec<_>>();
        let space = ExtensionType::try_calculate_account_len::<Mint>(&extension_types).unwrap();
        let mint_rent = self.rent.minimum_balance(space);

        let mut instructions = vec![system_instruction::create_account(
            &self.context.payer.pubkey(),
            &mint_keypair.pubkey(),
            mint_rent,
            space as u64,
            &spl_token_2022::id(),
        )];

        for params in extension_initialization_params {
            instructions.push(
                params
                    .instruction(&spl_token_2022::id(), &mint_keypair.pubkey())
                    .unwrap(),
            );
        }
        instructions.push(
            spl_token_2022::instruction::initialize_mint(
                &spl_token_2022::id(),
                &mint_keypair.pubkey(),
                mint_authority,
                freeze_authority,
                0,
            )
            .unwrap(),
        );
        self.process_transaction(&instructions, Some(&[mint_keypair]))
            .await
            .unwrap();
    }

    pub async fn create_mint_2022_transfer_hook(
        &mut self,
        mint_keypair: &Keypair,
        mint_authority: &Pubkey,
        program_id: &Pubkey,
        freeze_authority: Option<&Pubkey>,
    ) {
        let extension_initialization_params = vec![ExtensionInitializationParams::TransferHook {
            authority: Some(*mint_authority),
            program_id: Some(*program_id),
        }];

        let extension_types = extension_initialization_params
            .iter()
            .map(|e| e.extension())
            .collect::<Vec<_>>();
        let space = ExtensionType::try_calculate_account_len::<Mint>(&extension_types).unwrap();
        let mint_rent = self.rent.minimum_balance(space);

        let mut instructions = vec![system_instruction::create_account(
            &self.context.payer.pubkey(),
            &mint_keypair.pubkey(),
            mint_rent,
            space as u64,
            &spl_token_2022::id(),
        )];

        for params in extension_initialization_params {
            instructions.push(
                params
                    .instruction(&spl_token_2022::id(), &mint_keypair.pubkey())
                    .unwrap(),
            );
        }
        instructions.push(
            spl_token_2022::instruction::initialize_mint(
                &spl_token_2022::id(),
                &mint_keypair.pubkey(),
                mint_authority,
                freeze_authority,
                0,
            )
            .unwrap(),
        );
        self.process_transaction(&instructions, Some(&[mint_keypair]))
            .await
            .unwrap();
    }

    pub async fn initialize_transfer_hook_account_metas(
        &mut self,
        mint_address: &Pubkey,
        mint_authority: &Keypair,
        program_id: &Pubkey,
        source: &Pubkey,
        destination: &Pubkey,
        writable_pubkey: &Pubkey,
        amount: u64,
    ) -> Vec<AccountMeta> {
        let extra_account_metas_address =
            get_extra_account_metas_address(&mint_address, &program_id);

        let init_extra_account_metas = [
            ExtraAccountMeta::new_with_pubkey(&sysvar::instructions::id(), false, false).unwrap(),
            ExtraAccountMeta::new_with_pubkey(&mint_authority.pubkey(), false, false).unwrap(),
            ExtraAccountMeta::new_with_seeds(
                &[
                    Seed::Literal {
                        bytes: b"seed-prefix".to_vec(),
                    },
                    Seed::AccountKey { index: 0 },
                ],
                false,
                true,
            )
            .unwrap(),
            ExtraAccountMeta::new_with_seeds(
                &[
                    Seed::InstructionData {
                        index: 8,  // After instruction discriminator
                        length: 8, // `u64` (amount)
                    },
                    Seed::AccountKey { index: 2 },
                ],
                false,
                true,
            )
            .unwrap(),
            ExtraAccountMeta::new_with_pubkey(&writable_pubkey, false, true).unwrap(),
        ];

        let extra_pda_1 = Pubkey::find_program_address(
            &[
                b"seed-prefix",  // Literal prefix
                source.as_ref(), // Account at index 0
            ],
            &program_id,
        )
        .0;
        let extra_pda_2 = Pubkey::find_program_address(
            &[
                &amount.to_le_bytes(), // Instruction data bytes 8 to 16
                destination.as_ref(),  // Account at index 2
            ],
            &program_id,
        )
        .0;

        let extra_account_metas = [
            AccountMeta::new(extra_account_metas_address, false),
            AccountMeta::new(*program_id, false),
            AccountMeta::new_readonly(sysvar::instructions::id(), false),
            AccountMeta::new_readonly(mint_authority.pubkey(), false),
            AccountMeta::new(extra_pda_1, false),
            AccountMeta::new(extra_pda_2, false),
            AccountMeta::new(*writable_pubkey, false),
        ];

        let rent = self.context.banks_client.get_rent().await.unwrap();
        let rent_lamports = rent.minimum_balance(
            ExtraAccountMetaList::size_of(init_extra_account_metas.len()).unwrap(),
        );

        let transaction = Transaction::new_signed_with_payer(
            &[
                system_instruction::transfer(
                    &self.context.payer.pubkey(),
                    &extra_account_metas_address,
                    rent_lamports,
                ),
                initialize_extra_account_meta_list(
                    &program_id,
                    &extra_account_metas_address,
                    &mint_address,
                    &mint_authority.pubkey(),
                    &init_extra_account_metas,
                ),
            ],
            Some(&self.context.payer.pubkey()),
            &[&self.context.payer, &mint_authority],
            self.context.last_blockhash,
        );

        self.context
            .banks_client
            .process_transaction(transaction)
            .await
            .unwrap();

        extra_account_metas.to_vec()
    }

    pub async fn update_transfer_hook_account_metas(
        &mut self,
        mint_address: &Pubkey,
        mint_authority: &Keypair,
        program_id: &Pubkey,
        source: &Pubkey,
        destination: &Pubkey,
        updated_writable_pubkey: &Pubkey,
        amount: u64,
    ) -> Vec<AccountMeta> {
        let extra_account_metas_address =
            get_extra_account_metas_address(&mint_address, &program_id);

        let updated_extra_account_metas = [
            ExtraAccountMeta::new_with_pubkey(&sysvar::instructions::id(), false, false).unwrap(),
            ExtraAccountMeta::new_with_pubkey(&mint_authority.pubkey(), false, false).unwrap(),
            ExtraAccountMeta::new_with_seeds(
                &[
                    Seed::Literal {
                        bytes: b"updated-seed-prefix".to_vec(),
                    },
                    Seed::AccountKey { index: 0 },
                ],
                false,
                true,
            )
            .unwrap(),
            ExtraAccountMeta::new_with_seeds(
                &[
                    Seed::InstructionData {
                        index: 8,  // After instruction discriminator
                        length: 8, // `u64` (amount)
                    },
                    Seed::AccountKey { index: 2 },
                ],
                false,
                true,
            )
            .unwrap(),
            ExtraAccountMeta::new_with_pubkey(&updated_writable_pubkey, false, true).unwrap(),
        ];

        let extra_pda_1 = Pubkey::find_program_address(
            &[
                b"updated-seed-prefix", // Literal prefix
                source.as_ref(),        // Account at index 0
            ],
            &program_id,
        )
        .0;
        let extra_pda_2 = Pubkey::find_program_address(
            &[
                &amount.to_le_bytes(), // Instruction data bytes 8 to 16
                destination.as_ref(),  // Account at index 2
            ],
            &program_id,
        )
        .0;

        let extra_account_metas = [
            AccountMeta::new(extra_account_metas_address, false),
            AccountMeta::new(*program_id, false),
            AccountMeta::new_readonly(sysvar::instructions::id(), false),
            AccountMeta::new_readonly(mint_authority.pubkey(), false),
            AccountMeta::new(extra_pda_1, false),
            AccountMeta::new(extra_pda_2, false),
            AccountMeta::new(*updated_writable_pubkey, false),
        ];

        let rent = self.context.banks_client.get_rent().await.unwrap();
        let rent_lamports = rent.minimum_balance(
            ExtraAccountMetaList::size_of(updated_extra_account_metas.len()).unwrap(),
        );
        let transaction = Transaction::new_signed_with_payer(
            &[
                system_instruction::transfer(
                    &self.context.payer.pubkey(),
                    &extra_account_metas_address,
                    rent_lamports,
                ),
                update_extra_account_meta_list(
                    &program_id,
                    &extra_account_metas_address,
                    &mint_address,
                    &mint_authority.pubkey(),
                    &updated_extra_account_metas,
                ),
            ],
            Some(&self.context.payer.pubkey()),
            &[&self.context.payer, &mint_authority],
            self.context.last_blockhash,
        );

        self.context
            .banks_client
            .process_transaction(transaction)
            .await
            .unwrap();

        extra_account_metas.to_vec()
    }
    /// Sets spl-token program account (Mint or TokenAccount) authority
    pub async fn set_spl_token_account_authority(
        &mut self,
        account: &Pubkey,
        account_authority: &Keypair,
        new_authority: Option<&Pubkey>,
        authority_type: AuthorityType,
    ) {
        let set_authority_ix = set_authority(
            &spl_token::id(),
            account,
            new_authority,
            authority_type,
            &account_authority.pubkey(),
            &[],
        )
        .unwrap();

        self.process_transaction(&[set_authority_ix], Some(&[account_authority]))
            .await
            .unwrap();
    }

    /// Sets spl-token-2022 program account (Mint or TokenAccount) authority
    pub async fn set_spl_token_2022_account_authority(
        &mut self,
        account: &Pubkey,
        account_authority: &Keypair,
        new_authority: Option<&Pubkey>,
        authority_type: spl_token_2022::instruction::AuthorityType,
    ) {
        let set_authority_ix = spl_token_2022::instruction::set_authority(
            &spl_token_2022::id(),
            account,
            new_authority,
            authority_type,
            &account_authority.pubkey(),
            &[],
        )
        .unwrap();

        self.process_transaction(&[set_authority_ix], Some(&[account_authority]))
            .await
            .unwrap();
    }

    #[allow(dead_code)]
    pub async fn create_empty_token_account(
        &mut self,
        token_account_keypair: &Keypair,
        token_mint: &Pubkey,
        owner: &Pubkey,
    ) {
        let create_account_instruction = system_instruction::create_account(
            &self.context.payer.pubkey(),
            &token_account_keypair.pubkey(),
            self.rent
                .minimum_balance(spl_token::state::Account::get_packed_len()),
            spl_token::state::Account::get_packed_len() as u64,
            &spl_token::id(),
        );

        let initialize_account_instruction = spl_token::instruction::initialize_account(
            &spl_token::id(),
            &token_account_keypair.pubkey(),
            token_mint,
            owner,
        )
        .unwrap();

        self.process_transaction(
            &[create_account_instruction, initialize_account_instruction],
            Some(&[token_account_keypair]),
        )
        .await
        .unwrap();
    }

    #[allow(dead_code)]
    pub async fn create_empty_token_2022_account(
        &mut self,
        token_account_keypair: &Keypair,
        token_mint: &Pubkey,
        owner: &Pubkey,
    ) {
        let create_account_instruction = system_instruction::create_account(
            &self.context.payer.pubkey(),
            &token_account_keypair.pubkey(),
            self.rent
                .minimum_balance(spl_token_2022::state::Account::get_packed_len()),
            spl_token_2022::state::Account::get_packed_len() as u64,
            &spl_token_2022::id(),
        );

        let initialize_account_instruction = spl_token_2022::instruction::initialize_account(
            &spl_token_2022::id(),
            &token_account_keypair.pubkey(),
            token_mint,
            owner,
        )
        .unwrap();

        self.process_transaction(
            &[create_account_instruction, initialize_account_instruction],
            Some(&[token_account_keypair]),
        )
        .await
        .unwrap();
    }

    #[allow(dead_code)]
    pub async fn with_token_account(
        &mut self,
        token_mint: &Pubkey,
        owner: &Pubkey,
        token_mint_authority: &Keypair,
        amount: u64,
    ) -> TokenAccountCookie {
        let token_account_keypair = Keypair::new();

        self.create_empty_token_account(&token_account_keypair, token_mint, owner)
            .await;

        self.mint_tokens(
            token_mint,
            token_mint_authority,
            &token_account_keypair.pubkey(),
            amount,
        )
        .await;

        TokenAccountCookie {
            address: token_account_keypair.pubkey(),
        }
    }

    #[allow(dead_code)]
    pub async fn with_token_2022_account(
        &mut self,
        token_mint: &Pubkey,
        owner: &Pubkey,
        token_mint_authority: &Keypair,
        amount: u64,
    ) -> TokenAccountCookie {
        let token_account_keypair = Keypair::new();

        self.create_empty_token_2022_account(&token_account_keypair, token_mint, owner)
            .await;

        self.mint_2022_tokens(
            token_mint,
            token_mint_authority,
            &token_account_keypair.pubkey(),
            amount,
        )
        .await;

        TokenAccountCookie {
            address: token_account_keypair.pubkey(),
        }
    }

    pub async fn transfer_sol(&mut self, to_account: &Pubkey, lamports: u64) {
        let transfer_ix = system_instruction::transfer(&self.payer.pubkey(), to_account, lamports);

        self.process_transaction(&[transfer_ix], None)
            .await
            .unwrap();
    }

    pub async fn mint_tokens(
        &mut self,
        token_mint: &Pubkey,
        token_mint_authority: &Keypair,
        token_account: &Pubkey,
        amount: u64,
    ) {
        let mint_instruction = spl_token::instruction::mint_to(
            &spl_token::id(),
            token_mint,
            token_account,
            &token_mint_authority.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        self.process_transaction(&[mint_instruction], Some(&[token_mint_authority]))
            .await
            .unwrap();
    }

    pub async fn mint_2022_tokens(
        &mut self,
        token_mint: &Pubkey,
        token_mint_authority: &Keypair,
        token_account: &Pubkey,
        amount: u64,
    ) {
        let mint_instruction = spl_token_2022::instruction::mint_to(
            &spl_token_2022::id(),
            token_mint,
            token_account,
            &token_mint_authority.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        self.process_transaction(&[mint_instruction], Some(&[token_mint_authority]))
            .await
            .unwrap();
    }

    #[allow(dead_code)]
    pub async fn create_token_account_with_transfer_authority(
        &mut self,
        token_account_keypair: &Keypair,
        token_mint: &Pubkey,
        token_mint_authority: &Keypair,
        amount: u64,
        owner: &Keypair,
        transfer_authority: &Pubkey,
    ) {
        let create_account_instruction = system_instruction::create_account(
            &self.context.payer.pubkey(),
            &token_account_keypair.pubkey(),
            self.rent
                .minimum_balance(spl_token::state::Account::get_packed_len()),
            spl_token::state::Account::get_packed_len() as u64,
            &spl_token::id(),
        );

        let initialize_account_instruction = spl_token::instruction::initialize_account(
            &spl_token::id(),
            &token_account_keypair.pubkey(),
            token_mint,
            &owner.pubkey(),
        )
        .unwrap();

        let mint_instruction = spl_token::instruction::mint_to(
            &spl_token::id(),
            token_mint,
            &token_account_keypair.pubkey(),
            &token_mint_authority.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        let approve_instruction = spl_token::instruction::approve(
            &spl_token::id(),
            &token_account_keypair.pubkey(),
            transfer_authority,
            &owner.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        self.process_transaction(
            &[
                create_account_instruction,
                initialize_account_instruction,
                mint_instruction,
                approve_instruction,
            ],
            Some(&[token_account_keypair, token_mint_authority, owner]),
        )
        .await
        .unwrap();
    }

    #[allow(dead_code)]
    pub async fn create_token_2022_account_with_transfer_authority(
        &mut self,
        token_account_keypair: &Keypair,
        token_mint: &Pubkey,
        token_mint_authority: &Keypair,
        amount: u64,
        owner: &Keypair,
        transfer_authority: &Pubkey,
    ) {
        let create_account_instruction = system_instruction::create_account(
            &self.context.payer.pubkey(),
            &token_account_keypair.pubkey(),
            self.rent
                .minimum_balance(spl_token_2022::state::Account::get_packed_len()),
            spl_token_2022::state::Account::get_packed_len() as u64,
            &spl_token_2022::id(),
        );

        let initialize_account_instruction = spl_token_2022::instruction::initialize_account(
            &spl_token_2022::id(),
            &token_account_keypair.pubkey(),
            token_mint,
            &owner.pubkey(),
        )
        .unwrap();

        let mint_instruction = spl_token_2022::instruction::mint_to(
            &spl_token_2022::id(),
            token_mint,
            &token_account_keypair.pubkey(),
            &token_mint_authority.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        let approve_instruction = spl_token_2022::instruction::approve(
            &spl_token_2022::id(),
            &token_account_keypair.pubkey(),
            transfer_authority,
            &owner.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        self.process_transaction(
            &[
                create_account_instruction,
                initialize_account_instruction,
                mint_instruction,
                approve_instruction,
            ],
            Some(&[token_account_keypair, token_mint_authority, owner]),
        )
        .await
        .unwrap();
    }

    #[allow(dead_code)]
    pub async fn create_token_2022_account_with_transfer_authority_with_transfer_fees(
        &mut self,
        token_account_keypair: &Keypair,
        token_mint: &Pubkey,
        token_mint_authority: &Keypair,
        amount: u64,
        owner: &Keypair,
        transfer_authority: &Pubkey,
    ) {
        let space = ExtensionType::try_calculate_account_len::<Mint>(&[
            spl_token_2022::extension::ExtensionType::TransferFeeConfig,
        ])
        .unwrap();
        let mint_rent = self.rent.minimum_balance(space);

        let create_account_instruction = system_instruction::create_account(
            &self.context.payer.pubkey(),
            &token_account_keypair.pubkey(),
            mint_rent,
            space as u64,
            &spl_token_2022::id(),
        );

        let initialize_account_instruction = spl_token_2022::instruction::initialize_account(
            &spl_token_2022::id(),
            &token_account_keypair.pubkey(),
            token_mint,
            &owner.pubkey(),
        )
        .unwrap();

        let mint_instruction = spl_token_2022::instruction::mint_to(
            &spl_token_2022::id(),
            token_mint,
            &token_account_keypair.pubkey(),
            &token_mint_authority.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        let approve_instruction = spl_token_2022::instruction::approve(
            &spl_token_2022::id(),
            &token_account_keypair.pubkey(),
            transfer_authority,
            &owner.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        self.process_transaction(
            &[
                create_account_instruction,
                initialize_account_instruction,
                mint_instruction,
                approve_instruction,
            ],
            Some(&[token_account_keypair, token_mint_authority, owner]),
        )
        .await
        .unwrap();
    }

    #[allow(dead_code)]
    pub async fn create_token_2022_account_with_transfer_authority_with_transfer_hooks(
        &mut self,
        token_account_keypair: &Keypair,
        token_mint: &Pubkey,
        token_mint_authority: &Keypair,
        amount: u64,
        owner: &Keypair,
        transfer_authority: &Pubkey,
        program_id: &Pubkey,
    ) {
        let extension_initialization_params = vec![ExtensionInitializationParams::TransferHook {
            authority: Some(token_mint_authority.pubkey()),
            program_id: Some(*program_id),
        }];

        let extension_types = extension_initialization_params
            .iter()
            .map(|e| e.extension())
            .collect::<Vec<_>>();
        let space = ExtensionType::try_calculate_account_len::<Mint>(&extension_types).unwrap();
        let mint_rent = self.rent.minimum_balance(space);

        let create_account_instruction = system_instruction::create_account(
            &self.context.payer.pubkey(),
            &token_account_keypair.pubkey(),
            mint_rent,
            space as u64,
            &spl_token_2022::id(),
        );

        let initialize_account_instruction = spl_token_2022::instruction::initialize_account(
            &spl_token_2022::id(),
            &token_account_keypair.pubkey(),
            token_mint,
            &owner.pubkey(),
        )
        .unwrap();

        let mint_instruction = spl_token_2022::instruction::mint_to(
            &spl_token_2022::id(),
            token_mint,
            &token_account_keypair.pubkey(),
            &token_mint_authority.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        let approve_instruction = spl_token_2022::instruction::approve(
            &spl_token_2022::id(),
            &token_account_keypair.pubkey(),
            transfer_authority,
            &owner.pubkey(),
            &[],
            amount,
        )
        .unwrap();

        self.process_transaction(
            &[
                create_account_instruction,
                initialize_account_instruction,
                mint_instruction,
                approve_instruction,
            ],
            Some(&[token_account_keypair, token_mint_authority, owner]),
        )
        .await
        .unwrap();
    }

    #[allow(dead_code)]
    pub async fn get_clock(&mut self) -> Clock {
        self.get_bincode_account::<Clock>(&sysvar::clock::id())
            .await
    }

    #[allow(dead_code)]
    pub async fn get_bincode_account<T: serde::de::DeserializeOwned>(
        &mut self,
        address: &Pubkey,
    ) -> T {
        self.context
            .banks_client
            .get_account(*address)
            .await
            .unwrap()
            .map(|a| deserialize::<T>(a.data.borrow()).unwrap())
            .unwrap_or_else(|| panic!("GET-TEST-ACCOUNT-ERROR: Account {}", address))
    }

    /// TODO: Add to SDK
    pub async fn get_borsh_account<T: BorshDeserialize>(&mut self, address: &Pubkey) -> T {
        self.get_account(address)
            .await
            .map(|a| try_from_slice_unchecked(&a.data).unwrap())
            .unwrap_or_else(|| panic!("GET-TEST-ACCOUNT-ERROR: Account {} not found", address))
    }

    /// Overrides or creates Borsh serialized account with arbitrary account
    /// data subverting normal runtime checks
    pub fn set_borsh_account<T: BorshSerialize>(
        &mut self,
        program_id: &Pubkey,
        address: &Pubkey,
        account: &T,
    ) {
        let mut account_data = vec![];
        borsh::to_writer(&mut account_data, &account).unwrap();

        let data = AccountSharedData::create(
            self.rent.minimum_balance(account_data.len()),
            account_data,
            *program_id,
            false,
            Epoch::default(),
        );

        self.context.set_account(address, &data);
    }

    /// Removes an account by setting its data to empty and owner to system
    /// subverting normal runtime checks
    pub fn remove_account(&mut self, address: &Pubkey) {
        let data =
            AccountSharedData::create(0, vec![], system_program::id(), false, Epoch::default());

        self.context.set_account(address, &data);
    }

    #[allow(dead_code)]
    pub async fn get_account(&mut self, address: &Pubkey) -> Option<Account> {
        self.context
            .banks_client
            .get_account(*address)
            .await
            .unwrap()
    }
}
