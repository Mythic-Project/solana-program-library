#![cfg(feature = "test-sbf")]

mod program_test;

use {
    program_test::*,
    solana_program_test::tokio,
    solana_sdk::{signature::Keypair, signer::Signer},
    spl_governance::{
        error::GovernanceError,
        state::{
            enums::{ProposalState, TransactionExecutionStatus},
            native_treasury::get_native_treasury_address,
        },
        tools::{spl_token::inline_spl_token, transaction_message::TransactionMessage},
    },
    versioned_transaction_ext::VaultTransactionMessageExt,
};

#[tokio::test]
async fn test_execute_mint_versioned_transaction() {
    // Arrange
    let mut governance_test = GovernanceProgramTest::start_new().await;

    let realm_cookie = governance_test.with_realm().await;
    let governed_account_cookie = governance_test.with_governed_account().await;

    let token_owner_record_cookie = governance_test
        .with_community_token_deposit(&realm_cookie)
        .await
        .unwrap();

    let mut governance_cookie = governance_test
        .with_governance(
            &realm_cookie,
            &governed_account_cookie,
            &token_owner_record_cookie,
        )
        .await
        .unwrap();

    let governed_mint_cookie = governance_test
        .with_governed_mint_governed_authority(&governance_cookie)
        .await;

    let mut proposal_cookie = governance_test
        .with_proposal(&token_owner_record_cookie, &mut governance_cookie)
        .await
        .unwrap();

    let signatory_record_cookie = governance_test
        .with_signatory(
            &proposal_cookie,
            &governance_cookie,
            &token_owner_record_cookie,
        )
        .await
        .unwrap();

    let treasury_address =
        get_native_treasury_address(&governance_test.program_id, &governance_cookie.address);

    let token_account_keypair = Keypair::new();
    governance_test
        .bench
        .create_empty_token_account(
            &token_account_keypair,
            &governed_mint_cookie.address,
            &governance_test.bench.payer.pubkey(),
        )
        .await;

    let instruction = spl_token_2022::instruction::mint_to(
        &inline_spl_token::id(),
        &governed_mint_cookie.address,
        &token_account_keypair.pubkey(),
        &governance_cookie.address,
        &[],
        10,
    )
    .unwrap();

    let transaction_message = <TransactionMessage as VaultTransactionMessageExt>::try_compile(
        &governance_cookie.address,
        &[instruction],
        &[],
    )
    .unwrap();
    // Act
    let transaction_message_bytes = borsh::to_vec(&transaction_message).unwrap();

    let proposal_transaction_cookie = governance_test
        .with_insert_versioned_transaction(
            &mut proposal_cookie,
            &token_owner_record_cookie,
            0,
            0,
            None,
            transaction_message_bytes,
        )
        .await
        .unwrap();

    governance_test
        .sign_off_proposal(&proposal_cookie, &signatory_record_cookie)
        .await
        .unwrap();

    governance_test
        .with_cast_yes_no_vote(&proposal_cookie, &token_owner_record_cookie, YesNoVote::Yes)
        .await
        .unwrap();

    // Advance timestamp past hold_up_time
    governance_test
        .advance_clock_by_min_timespan(
            governance_cookie
                .account
                .config
                .min_transaction_hold_up_time as u64,
        )
        .await;

    let clock = governance_test.bench.get_clock().await;

    // Act
    governance_test
        .with_execute_versioned_transaction(
            &proposal_cookie,
            &proposal_transaction_cookie,
            transaction_message,
            0,
            0,
            &treasury_address,
            &proposal_cookie.account.governance,
            &[],
        )
        .await
        .unwrap();

    // Assert

    let proposal_account = governance_test
        .get_proposal_account(&proposal_cookie.address)
        .await;

    let yes_option = proposal_account.options.first().unwrap();

    assert_eq!(1, yes_option.transactions_executed_count);
    assert_eq!(ProposalState::Completed, proposal_account.state);
    assert_eq!(Some(clock.unix_timestamp), proposal_account.closed_at);
    assert_eq!(Some(clock.unix_timestamp), proposal_account.executing_at);

    let proposal_transaction_account = governance_test
        .get_proposal_versioned_transaction_account(&proposal_transaction_cookie.address)
        .await;

    assert_eq!(
        Some(clock.unix_timestamp),
        proposal_transaction_account.executed_at
    );

    assert_eq!(
        TransactionExecutionStatus::Success,
        proposal_transaction_account.execution_status
    );

    let instruction_token_account = governance_test
        .get_token_account(&token_account_keypair.pubkey())
        .await;

    assert_eq!(10, instruction_token_account.amount);
}

#[tokio::test]
async fn test_execute_transfer_versioned_transaction() {
    // Arrange
    let mut governance_test = GovernanceProgramTest::start_new().await;

    let realm_cookie = governance_test.with_realm().await;
    let governed_account_cookie = governance_test.with_governed_account().await;

    let token_owner_record_cookie = governance_test
        .with_community_token_deposit(&realm_cookie)
        .await
        .unwrap();

    let mut governance_cookie = governance_test
        .with_governance(
            &realm_cookie,
            &governed_account_cookie,
            &token_owner_record_cookie,
        )
        .await
        .unwrap();

    let governed_token_account_cookie = governance_test
        .with_governed_token_governed_authority(&governance_cookie)
        .await;

    let mut proposal_cookie = governance_test
        .with_proposal(&token_owner_record_cookie, &mut governance_cookie)
        .await
        .unwrap();

    let signatory_record_cookie = governance_test
        .with_signatory(
            &proposal_cookie,
            &governance_cookie,
            &token_owner_record_cookie,
        )
        .await
        .unwrap();

    let treasury_address =
        get_native_treasury_address(&governance_test.program_id, &governance_cookie.address);

    let token_account_keypair = Keypair::new();
    governance_test
        .bench
        .create_empty_token_account(
            &token_account_keypair,
            &governed_token_account_cookie.token_mint,
            &governance_test.bench.payer.pubkey(),
        )
        .await;

    #[allow(deprecated)]
    let instruction = spl_token_2022::instruction::transfer(
        &inline_spl_token::id(),
        &governed_token_account_cookie.address,
        &token_account_keypair.pubkey(),
        &proposal_cookie.account.governance,
        &[],
        15,
    )
    .unwrap();

    let transaction_message = <TransactionMessage as VaultTransactionMessageExt>::try_compile(
        &proposal_cookie.account.governance,
        &[instruction],
        &[],
    )
    .unwrap();
    // Act
    let transaction_message_bytes = borsh::to_vec(&transaction_message).unwrap();

    let proposal_transaction_cookie = governance_test
        .with_insert_versioned_transaction(
            &mut proposal_cookie,
            &token_owner_record_cookie,
            0,
            0,
            None,
            transaction_message_bytes,
        )
        .await
        .unwrap();

    governance_test
        .sign_off_proposal(&proposal_cookie, &signatory_record_cookie)
        .await
        .unwrap();

    governance_test
        .with_cast_yes_no_vote(&proposal_cookie, &token_owner_record_cookie, YesNoVote::Yes)
        .await
        .unwrap();

    // Advance timestamp past hold_up_time
    governance_test
        .advance_clock_by_min_timespan(
            governance_cookie
                .account
                .config
                .min_transaction_hold_up_time as u64,
        )
        .await;

    let clock = governance_test.bench.get_clock().await;

    // Act
    governance_test
        .with_execute_versioned_transaction(
            &proposal_cookie,
            &proposal_transaction_cookie,
            transaction_message,
            0,
            0,
            &treasury_address,
            &proposal_cookie.account.governance,
            &[],
        )
        .await
        .unwrap();

    // Assert

    let proposal_account = governance_test
        .get_proposal_account(&proposal_cookie.address)
        .await;

    let yes_option = proposal_account.options.first().unwrap();

    assert_eq!(1, yes_option.transactions_executed_count);
    assert_eq!(ProposalState::Completed, proposal_account.state);
    assert_eq!(Some(clock.unix_timestamp), proposal_account.closed_at);
    assert_eq!(Some(clock.unix_timestamp), proposal_account.executing_at);

    let proposal_transaction_account = governance_test
        .get_proposal_versioned_transaction_account(&proposal_transaction_cookie.address)
        .await;

    assert_eq!(
        Some(clock.unix_timestamp),
        proposal_transaction_account.executed_at
    );

    assert_eq!(
        TransactionExecutionStatus::Success,
        proposal_transaction_account.execution_status
    );

    let instruction_token_account = governance_test
        .get_token_account(&token_account_keypair.pubkey())
        .await;

    assert_eq!(15, instruction_token_account.amount);
}

#[tokio::test]
async fn test_execute_versioned_transaction_with_create_proposal_and_execute_in_single_slot_error()
{
    // Arrange
    let mut governance_test = GovernanceProgramTest::start_new().await;

    let realm_cookie = governance_test.with_realm().await;
    let governed_account_cookie = governance_test.with_governed_account().await;

    let token_owner_record_cookie = governance_test
        .with_community_token_deposit(&realm_cookie)
        .await
        .unwrap();

    let mut governance_config = governance_test.get_default_governance_config();
    governance_config.min_transaction_hold_up_time = 0;

    let mut governance_cookie = governance_test
        .with_governance_using_config(
            &realm_cookie,
            &governed_account_cookie,
            &token_owner_record_cookie,
            &governance_config,
        )
        .await
        .unwrap();

    let governed_mint_cookie = governance_test.with_governed_mint().await;

    let mut proposal_cookie = governance_test
        .with_proposal(&token_owner_record_cookie, &mut governance_cookie)
        .await
        .unwrap();

    let signatory_record_cookie = governance_test
        .with_signatory(
            &proposal_cookie,
            &governance_cookie,
            &token_owner_record_cookie,
        )
        .await
        .unwrap();

    let treasury_address =
        get_native_treasury_address(&governance_test.program_id, &governance_cookie.address);

    let token_account_keypair = Keypair::new();
    governance_test
        .bench
        .create_empty_token_account(
            &token_account_keypair,
            &governed_mint_cookie.address,
            &governance_test.bench.payer.pubkey(),
        )
        .await;

    let instruction = spl_token_2022::instruction::mint_to(
        &inline_spl_token::id(),
        &governed_mint_cookie.address,
        &token_account_keypair.pubkey(),
        &proposal_cookie.account.governance,
        &[],
        10,
    )
    .unwrap();

    let transaction_message = <TransactionMessage as VaultTransactionMessageExt>::try_compile(
        &proposal_cookie.account.governance,
        &[instruction],
        &[],
    )
    .unwrap();
    // Act
    let transaction_message_bytes = borsh::to_vec(&transaction_message).unwrap();

    let proposal_transaction_cookie = governance_test
        .with_insert_versioned_transaction(
            &mut proposal_cookie,
            &token_owner_record_cookie,
            0,
            0,
            None,
            transaction_message_bytes,
        )
        .await
        .unwrap();

    governance_test
        .sign_off_proposal(&proposal_cookie, &signatory_record_cookie)
        .await
        .unwrap();

    governance_test
        .with_cast_yes_no_vote(&proposal_cookie, &token_owner_record_cookie, YesNoVote::Yes)
        .await
        .unwrap();

    // Act
    let err = governance_test
        .with_execute_versioned_transaction(
            &proposal_cookie,
            &proposal_transaction_cookie,
            transaction_message,
            0,
            0,
            &treasury_address,
            &proposal_cookie.account.governance,
            &[],
        )
        .await
        .err()
        .unwrap();

    // Assert
    assert_eq!(
        err,
        GovernanceError::CannotExecuteTransactionWithinHoldUpTime.into()
    );
}

#[tokio::test]
async fn test_execute_mint_versioned_transaction_token_2022_with_extensions() {
    // Arrange
    let mut governance_test = GovernanceProgramTest::start_new().await;

    let realm_cookie = governance_test.with_realm().await;
    let governed_account_cookie = governance_test.with_governed_account().await;

    let token_owner_record_cookie = governance_test
        .with_community_token_deposit(&realm_cookie)
        .await
        .unwrap();

    let mut governance_cookie = governance_test
        .with_governance(
            &realm_cookie,
            &governed_account_cookie,
            &token_owner_record_cookie,
        )
        .await
        .unwrap();

    let governed_mint_cookie = governance_test
        .with_governed_mint_governed_authority_token_2022_with_extensions(&governance_cookie)
        .await;

    let mut proposal_cookie = governance_test
        .with_proposal(&token_owner_record_cookie, &mut governance_cookie)
        .await
        .unwrap();

    let signatory_record_cookie = governance_test
        .with_signatory(
            &proposal_cookie,
            &governance_cookie,
            &token_owner_record_cookie,
        )
        .await
        .unwrap();

    let treasury_address =
        get_native_treasury_address(&governance_test.program_id, &governance_cookie.address);

    let token_account_keypair = Keypair::new();
    governance_test
        .bench
        .create_empty_token_2022_account(
            &token_account_keypair,
            &governed_mint_cookie.address,
            &governance_test.bench.payer.pubkey(),
        )
        .await;

    let instruction = spl_token_2022::instruction::mint_to(
        &&spl_token_2022::id(),
        &governed_mint_cookie.address,
        &token_account_keypair.pubkey(),
        &governance_cookie.address,
        &[],
        10,
    )
    .unwrap();

    let transaction_message = <TransactionMessage as VaultTransactionMessageExt>::try_compile(
        &governance_cookie.address,
        &[instruction],
        &[],
    )
    .unwrap();
    // Act
    let transaction_message_bytes = borsh::to_vec(&transaction_message).unwrap();

    let proposal_transaction_cookie = governance_test
        .with_insert_versioned_transaction(
            &mut proposal_cookie,
            &token_owner_record_cookie,
            0,
            0,
            None,
            transaction_message_bytes,
        )
        .await
        .unwrap();

    governance_test
        .sign_off_proposal(&proposal_cookie, &signatory_record_cookie)
        .await
        .unwrap();

    governance_test
        .with_cast_yes_no_vote(&proposal_cookie, &token_owner_record_cookie, YesNoVote::Yes)
        .await
        .unwrap();

    // Advance timestamp past hold_up_time
    governance_test
        .advance_clock_by_min_timespan(
            governance_cookie
                .account
                .config
                .min_transaction_hold_up_time as u64,
        )
        .await;

    let clock = governance_test.bench.get_clock().await;

    // Act
    governance_test
        .with_execute_versioned_transaction(
            &proposal_cookie,
            &proposal_transaction_cookie,
            transaction_message,
            0,
            0,
            &treasury_address,
            &proposal_cookie.account.governance,
            &[],
        )
        .await
        .unwrap();

    // Assert

    let proposal_account = governance_test
        .get_proposal_account(&proposal_cookie.address)
        .await;

    let yes_option = proposal_account.options.first().unwrap();

    assert_eq!(1, yes_option.transactions_executed_count);
    assert_eq!(ProposalState::Completed, proposal_account.state);
    assert_eq!(Some(clock.unix_timestamp), proposal_account.closed_at);
    assert_eq!(Some(clock.unix_timestamp), proposal_account.executing_at);

    let proposal_transaction_account = governance_test
        .get_proposal_versioned_transaction_account(&proposal_transaction_cookie.address)
        .await;

    assert_eq!(
        Some(clock.unix_timestamp),
        proposal_transaction_account.executed_at
    );

    assert_eq!(
        TransactionExecutionStatus::Success,
        proposal_transaction_account.execution_status
    );

    let instruction_token_account = governance_test
        .get_token_account(&token_account_keypair.pubkey())
        .await;

    assert_eq!(10, instruction_token_account.amount);
}
