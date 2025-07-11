// Copyright (c) Zefchain Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Tests for how the runtime computes fees based on consumed resources.

#![allow(clippy::items_after_test_module)]

use std::{collections::BTreeSet, sync::Arc, vec};

use linera_base::{
    crypto::AccountPublicKey,
    data_types::{Amount, BlockHeight, OracleResponse},
    http,
    identifiers::{Account, AccountOwner, MessageId},
    vm::VmRuntime,
};
use linera_execution::{
    test_utils::{
        blob_oracle_responses, dummy_chain_description, ExpectedCall, RegisterMockApplication,
        SystemExecutionState,
    },
    ContractRuntime, ExecutionError, Message, MessageContext, ResourceControlPolicy,
    ResourceController, ResourceTracker, TransactionTracker,
};
use test_case::test_case;

/// Tests if the chain balance is updated based on the fees spent for consuming resources.
// Chain account only.
#[test_case(vec![], Amount::ZERO, None, None; "without any costs")]
#[test_case(vec![FeeSpend::Fuel(100)], Amount::from_tokens(1_000), None, None; "with only execution costs")]
#[test_case(vec![FeeSpend::Read(vec![0, 1], None)], Amount::from_tokens(1_000), None, None; "with only an empty read")]
#[test_case(
    vec![
        FeeSpend::Read(vec![0, 1], None),
        FeeSpend::Fuel(207),
    ],
    Amount::from_tokens(1_000),
    None,
    None;
    "with execution and an empty read"
)]
// Chain account and small owner account.
#[test_case(
    vec![FeeSpend::Fuel(100)],
    Amount::from_tokens(1_000),
    Some(Amount::from_tokens(1)),
    None;
    "with only execution costs and with owner account"
)]
#[test_case(
    vec![FeeSpend::Read(vec![0, 1], None)],
    Amount::from_tokens(1_000),
    Some(Amount::from_tokens(1)),
    None;
    "with only an empty read and with owner account"
)]
#[test_case(
    vec![
        FeeSpend::Read(vec![0, 1], None),
        FeeSpend::Fuel(207),
    ],
    Amount::from_tokens(1_000),
    Some(Amount::from_tokens(1)),
    None;
    "with execution and an empty read and with owner account"
)]
// Small chain account and larger owner account.
#[test_case(
    vec![FeeSpend::Fuel(100)],
    Amount::from_tokens(1),
    Some(Amount::from_tokens(1_000)),
    None;
    "with only execution costs and with larger owner account"
)]
#[test_case(
    vec![FeeSpend::Read(vec![0, 1], None)],
    Amount::from_tokens(1),
    Some(Amount::from_tokens(1_000)),
    None;
    "with only an empty read and with larger owner account"
)]
#[test_case(
    vec![
        FeeSpend::Read(vec![0, 1], None),
        FeeSpend::Fuel(207),
    ],
    Amount::from_tokens(1),
    Some(Amount::from_tokens(1_000)),
    None;
    "with execution and an empty read and with larger owner account"
)]
// Small chain account, small owner account, large grant.
#[test_case(
    vec![FeeSpend::Fuel(100)],
    Amount::from_tokens(2),
    Some(Amount::from_tokens(1)),
    Some(Amount::from_tokens(1_000));
    "with only execution costs and with owner account and grant"
)]
#[test_case(
    vec![FeeSpend::Read(vec![0, 1], None)],
    Amount::from_tokens(2),
    Some(Amount::from_tokens(1)),
    Some(Amount::from_tokens(1_000));
    "with only an empty read and with owner account and grant"
)]
#[test_case(
    vec![
        FeeSpend::Read(vec![0, 1], None),
        FeeSpend::Fuel(207),
    ],
    Amount::from_tokens(2),
    Some(Amount::from_tokens(1)),
    Some(Amount::from_tokens(1_000));
    "with execution and an empty read and with owner account and grant"
)]
#[test_case(
    vec![
        FeeSpend::QueryServiceOracle,
        FeeSpend::Runtime(32),
    ],
    Amount::from_tokens(2),
    Some(Amount::from_tokens(1)),
    Some(Amount::from_tokens(1_000));
    "with only a service oracle call"
)]
#[test_case(
    vec![
        FeeSpend::QueryServiceOracle,
        FeeSpend::QueryServiceOracle,
        FeeSpend::QueryServiceOracle,
        FeeSpend::Runtime(96),
    ],
    Amount::from_tokens(2),
    Some(Amount::from_tokens(1)),
    Some(Amount::from_tokens(1_000));
    "with three service oracle calls"
)]
#[test_case(
    vec![
        FeeSpend::Fuel(91),
        FeeSpend::QueryServiceOracle,
        FeeSpend::Fuel(11),
        FeeSpend::Read(vec![0, 1, 2], None),
        FeeSpend::QueryServiceOracle,
        FeeSpend::Fuel(57),
        FeeSpend::QueryServiceOracle,
        FeeSpend::Runtime(96),
    ],
    Amount::from_tokens(2),
    Some(Amount::from_tokens(1_000)),
    None;
    "with service oracle calls, fuel consumption and a read operation"
)]
#[test_case(
    vec![FeeSpend::HttpRequest],
    Amount::from_tokens(2),
    Some(Amount::from_tokens(1)),
    Some(Amount::from_tokens(1_000));
    "with one HTTP request"
)]
#[test_case(
    vec![
        FeeSpend::HttpRequest,
        FeeSpend::HttpRequest,
        FeeSpend::HttpRequest,
    ],
    Amount::from_tokens(2),
    Some(Amount::from_tokens(1)),
    Some(Amount::from_tokens(1_000));
    "with three HTTP requests"
)]
#[test_case(
    vec![
        FeeSpend::Fuel(11),
        FeeSpend::HttpRequest,
        FeeSpend::Read(vec![0, 1], None),
        FeeSpend::Fuel(23),
        FeeSpend::HttpRequest,
    ],
    Amount::from_tokens(2),
    Some(Amount::from_tokens(1)),
    Some(Amount::from_tokens(1_000));
    "with all fee spend operations"
)]
// TODO(#1601): Add more test cases
#[tokio::test]
async fn test_fee_consumption(
    spends: Vec<FeeSpend>,
    chain_balance: Amount,
    owner_balance: Option<Amount>,
    initial_grant: Option<Amount>,
) -> anyhow::Result<()> {
    let chain_description = dummy_chain_description(0);
    let chain_id = chain_description.id();
    let mut state = SystemExecutionState {
        description: Some(chain_description.clone()),
        ..SystemExecutionState::default()
    };
    let (application_id, application, blobs) = state.register_mock_application(0).await?;
    let mut view = state.into_view().await;

    let mut oracle_responses = blob_oracle_responses(blobs.iter());

    let signer = AccountOwner::from(AccountPublicKey::test_key(0));
    view.system.balance.set(chain_balance);
    if let Some(owner_balance) = owner_balance {
        view.system.balances.insert(&signer, owner_balance)?;
    }

    let prices = ResourceControlPolicy {
        wasm_fuel_unit: Amount::from_tokens(3),
        evm_fuel_unit: Amount::from_tokens(2),
        read_operation: Amount::from_tokens(3),
        write_operation: Amount::from_tokens(5),
        byte_runtime: Amount::from_millis(1),
        byte_read: Amount::from_tokens(7),
        byte_written: Amount::from_tokens(11),
        byte_stored: Amount::from_tokens(13),
        operation: Amount::from_tokens(17),
        operation_byte: Amount::from_tokens(19),
        message: Amount::from_tokens(23),
        message_byte: Amount::from_tokens(29),
        service_as_oracle_query: Amount::from_millis(31),
        http_request: Amount::from_tokens(37),
        maximum_wasm_fuel_per_block: 4_868_145_137,
        maximum_evm_fuel_per_block: 4_868_145_137,
        maximum_block_size: 41,
        maximum_service_oracle_execution_ms: 43,
        maximum_blob_size: 47,
        maximum_published_blobs: 53,
        maximum_bytecode_size: 59,
        maximum_block_proposal_size: 61,
        maximum_bytes_read_per_block: 67,
        maximum_bytes_written_per_block: 71,
        maximum_oracle_response_bytes: 73,
        maximum_http_response_bytes: 79,
        http_request_timeout_ms: 83,
        blob_read: Amount::from_tokens(89),
        blob_published: Amount::from_tokens(97),
        blob_byte_read: Amount::from_tokens(101),
        blob_byte_published: Amount::from_tokens(103),
        http_request_allow_list: BTreeSet::new(),
    };

    let consumed_fees = spends
        .iter()
        .map(|spend| spend.amount(&prices))
        .fold(Amount::ZERO, |sum, spent_fees| {
            sum.saturating_add(spent_fees)
        });

    let authenticated_signer = if owner_balance.is_some() {
        Some(signer)
    } else {
        None
    };
    let mut controller = ResourceController::new(
        Arc::new(prices),
        ResourceTracker::default(),
        authenticated_signer,
    );

    for spend in &spends {
        oracle_responses.extend(spend.expected_oracle_responses());
    }

    application.expect_call(ExpectedCall::execute_message(move |runtime, _operation| {
        for spend in spends {
            spend.execute(runtime)?;
        }
        Ok(())
    }));
    application.expect_call(ExpectedCall::default_finalize());

    let refund_grant_to = authenticated_signer
        .map(|owner| Account { chain_id, owner })
        .or(None);
    let context = MessageContext {
        chain_id,
        is_bouncing: false,
        authenticated_signer,
        refund_grant_to,
        height: BlockHeight(0),
        round: Some(0),
        message_id: MessageId::default(),
        timestamp: Default::default(),
    };
    let mut grant = initial_grant.unwrap_or_default();
    let mut txn_tracker = TransactionTracker::new_replaying(oracle_responses);
    view.execute_message(
        context,
        Message::User {
            application_id,
            bytes: vec![],
        },
        if initial_grant.is_some() {
            Some(&mut grant)
        } else {
            None
        },
        &mut txn_tracker,
        &mut controller,
    )
    .await?;

    let txn_outcome = txn_tracker.into_outcome()?;
    assert!(txn_outcome.outgoing_messages.is_empty());

    match initial_grant {
        None => {
            let (expected_chain_balance, expected_owner_balance) = if chain_balance >= consumed_fees
            {
                (chain_balance.saturating_sub(consumed_fees), owner_balance)
            } else {
                let Some(owner_balance) = owner_balance else {
                    panic!("execution should have failed earlier");
                };
                (
                    Amount::ZERO,
                    Some(
                        owner_balance
                            .saturating_add(chain_balance)
                            .saturating_sub(consumed_fees),
                    ),
                )
            };
            assert_eq!(*view.system.balance.get(), expected_chain_balance);
            assert_eq!(
                view.system.balances.get(&signer).await?,
                expected_owner_balance
            );
            assert_eq!(grant, Amount::ZERO);
        }
        Some(initial_grant) => {
            let (expected_grant, expected_owner_balance) = if initial_grant >= consumed_fees {
                (initial_grant.saturating_sub(consumed_fees), owner_balance)
            } else {
                let Some(owner_balance) = owner_balance else {
                    panic!("execution should have failed earlier");
                };
                (
                    Amount::ZERO,
                    Some(
                        owner_balance
                            .saturating_add(initial_grant)
                            .saturating_sub(consumed_fees),
                    ),
                )
            };
            assert_eq!(*view.system.balance.get(), chain_balance);
            assert_eq!(
                view.system.balances.get(&signer).await?,
                expected_owner_balance
            );
            assert_eq!(grant, expected_grant);
        }
    }

    Ok(())
}

/// A runtime operation that costs some amount of fees.
pub enum FeeSpend {
    /// Consume some execution fuel.
    Fuel(u64),
    /// Reads from storage.
    Read(Vec<u8>, Option<Vec<u8>>),
    /// Queries a service as an oracle.
    QueryServiceOracle,
    /// Performs an HTTP request.
    HttpRequest,
    /// Byte from runtime.
    Runtime(u32),
}

impl FeeSpend {
    /// Returns the [`OracleResponse`]s necessary for executing this runtime operation.
    pub fn expected_oracle_responses(&self) -> Vec<OracleResponse> {
        match self {
            FeeSpend::Fuel(_) | FeeSpend::Read(_, _) | FeeSpend::Runtime(_) => vec![],
            FeeSpend::QueryServiceOracle => {
                vec![OracleResponse::Service(vec![])]
            }
            FeeSpend::HttpRequest => vec![OracleResponse::Http(http::Response::ok([]))],
        }
    }

    /// The fee amount required for this runtime operation.
    pub fn amount(&self, policy: &ResourceControlPolicy) -> Amount {
        match self {
            FeeSpend::Fuel(units) => policy.wasm_fuel_unit.saturating_mul(*units as u128),
            FeeSpend::Read(_key, value) => {
                let value_read_fee = value
                    .as_ref()
                    .map(|value| Amount::from(value.len() as u128))
                    .unwrap_or(Amount::ZERO);

                policy.read_operation.saturating_add(value_read_fee)
            }
            FeeSpend::QueryServiceOracle => policy.service_as_oracle_query,
            FeeSpend::HttpRequest => policy.http_request,
            FeeSpend::Runtime(bytes) => policy.byte_runtime.saturating_mul(*bytes as u128),
        }
    }

    /// Executes the operation with the `runtime`
    pub fn execute(self, runtime: &mut impl ContractRuntime) -> Result<(), ExecutionError> {
        match self {
            FeeSpend::Fuel(units) => runtime.consume_fuel(units, VmRuntime::Wasm),
            FeeSpend::Runtime(_bytes) => Ok(()),
            FeeSpend::Read(key, value) => {
                let promise = runtime.read_value_bytes_new(key)?;
                let response = runtime.read_value_bytes_wait(&promise)?;
                assert_eq!(response, value);
                Ok(())
            }
            FeeSpend::QueryServiceOracle => {
                let application_id = runtime.application_id()?;
                runtime.query_service(application_id, vec![])?;
                Ok(())
            }
            FeeSpend::HttpRequest => {
                runtime.perform_http_request(http::Request::get("http://dummy.url"))?;
                Ok(())
            }
        }
    }
}
