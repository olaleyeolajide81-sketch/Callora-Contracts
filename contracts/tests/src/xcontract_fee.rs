//! Cross-contract call safety tests for the fee-deduction mechanism (issue #879).
//!
//! The vault's [`deduct`](callora_vault::CalloraVault::deduct) function reduces
//! the vault's internal balance, transfers USDC to the settlement contract, and
//! then makes a cross-contract call to
//! [`settlement::record_deduction`](callora_settlement::CalloraSettlement::record_deduction).
//!
//! These tests verify that when the settlement callee panics mid-call, the
//! failure is surfaced as an `Err` (never a hard host abort), and Soroban's
//! atomic-invocation guarantee holds end-to-end: no partial state and no
//! partial events survive from either side of the failed call.

extern crate std;

use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Events as _, Address, Env, IntoVal, Symbol,
    Val, Vec,
};

// ---------------------------------------------------------------------------
// Mock settlement contracts
// ---------------------------------------------------------------------------

/// Settlement that always panics. Simulates a downstream crash during
/// [`record_deduction`].
mod mock_panicking {
    use super::*;

    #[contract]
    pub struct PanickingSettlement;

    #[contractimpl]
    impl PanickingSettlement {
        /// Always panics; the return type only exists to match [`OkSettlement`].
        pub fn record_deduction(_env: Env, _amount: i128, _request_id: u64) -> i128 {
            panic!("PanickingSettlement: deliberate revert");
        }
    }
}
pub use mock_panicking::PanickingSettlement;

/// Settlement that succeeds. Used as the control case.
mod mock_ok {
    use super::*;

    #[contract]
    pub struct OkSettlement;

    #[contractimpl]
    impl OkSettlement {
        pub fn record_deduction(_env: Env, amount: i128, _request_id: u64) -> i128 {
            amount
        }
    }
}
pub use mock_ok::OkSettlement;

// ---------------------------------------------------------------------------
// Fee caller — mirrors the vault's `deduct` pattern
// ---------------------------------------------------------------------------

/// Minimal caller that mimics the vault's fee-deduction flow:
///
/// 1. Store caller state (`balance`, `hits`)
/// 2. Invoke settlement's [`record_deduction`]
/// 3. Emit a `Deducted` event only after the callee returns successfully
///
/// This mirrors the real pattern in
/// [`CalloraVault::deduct`](callora_vault::CalloraVault::deduct).
#[contract]
pub struct FeeCaller;

#[contracttype]
pub enum FeeCallerDataKey {
    Balance,
    Hits,
}

#[contractimpl]
impl FeeCaller {
    /// Initialises the caller with a starting `balance`.
    pub fn init(env: Env, balance: i128) {
        env.storage()
            .instance()
            .set(&FeeCallerDataKey::Balance, &balance);
    }

    /// Mirrors the vault's `deduct` pattern:
    /// - Reads & decrements `balance`
    /// - Invokes `settlement.record_deduction(amount, request_id)` via
    ///   [`invoke_contract`]
    /// - Publishes a `Deducted` event on success
    pub fn deduct(env: Env, settlement: Address, amount: i128, request_id: u64) -> i128 {
        let balance: i128 = env
            .storage()
            .instance()
            .get(&FeeCallerDataKey::Balance)
            .unwrap_or(0);
        let new_balance = balance.checked_sub(amount).expect("underflow");
        env.storage()
            .instance()
            .set(&FeeCallerDataKey::Balance, &new_balance);

        env.storage().instance().set(&FeeCallerDataKey::Hits, &1u32);

        // Cross-contract call — this is the point of failure we test.
        let args: Vec<Val> =
            Vec::from_array(&env, [amount.into_val(&env), request_id.into_val(&env)]);
        let _result: i128 =
            env.invoke_contract(&settlement, &Symbol::new(&env, "record_deduction"), args);

        env.events().publish((Symbol::new(&env, "deducted"),), ());

        new_balance
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn balance(env: &Env, caller_addr: &Address) -> Option<i128> {
    env.as_contract(caller_addr, || {
        env.storage().instance().get(&FeeCallerDataKey::Balance)
    })
}

fn hits(env: &Env, caller_addr: &Address) -> Option<u32> {
    env.as_contract(caller_addr, || {
        env.storage().instance().get(&FeeCallerDataKey::Hits)
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verifies that a panic in the settlement callee is surfaced as an `Err`
/// via the generated `try_deduct` client, rather than aborting the test host.
#[test]
fn settlement_panic_is_surfaced_as_err_not_host_abort() {
    let env = Env::default();
    let settlement_addr = env.register(PanickingSettlement, ());
    let caller_addr = env.register(FeeCaller, ());
    let caller_client = FeeCallerClient::new(&env, &caller_addr);

    caller_client.init(&1000);
    let result = caller_client.try_deduct(&settlement_addr, &100, &1);

    assert!(
        result.is_err(),
        "a panicking settlement must surface as Err, not abort the host"
    );
}

/// Verifies that when the settlement panics, all caller state changes and
/// events are rolled back atomically.
#[test]
fn settlement_panic_rolls_back_caller_state_and_events() {
    let env = Env::default();
    let settlement_addr = env.register(PanickingSettlement, ());
    let caller_addr = env.register(FeeCaller, ());
    let caller_client = FeeCallerClient::new(&env, &caller_addr);

    caller_client.init(&1000);
    let _ = caller_client.try_deduct(&settlement_addr, &100, &1);

    // No events should survive a reverted cross-contract call.
    assert!(
        env.events().all().is_empty(),
        "no event should survive a reverted cross-contract call"
    );

    // State written before the cross-contract call must not persist.
    assert_eq!(
        balance(&env, &caller_addr),
        Some(1000),
        "balance must be unchanged after a reverted cross-call"
    );
    assert_eq!(
        hits(&env, &caller_addr),
        None,
        "state written before a reverted cross-call must not persist"
    );
}

/// Baseline control test: a successful cross-contract call persists state
/// and emits the expected event.
#[test]
fn successful_fee_deduct_persists_state_and_emits_event() {
    let env = Env::default();
    let settlement_addr = env.register(OkSettlement, ());
    let caller_addr = env.register(FeeCaller, ());
    let caller_client = FeeCallerClient::new(&env, &caller_addr);

    caller_client.init(&1000);
    let remaining = caller_client.deduct(&settlement_addr, &300, &42);

    assert_eq!(
        remaining, 700,
        "remaining balance should be initial - deducted"
    );
    assert_eq!(
        balance(&env, &caller_addr),
        Some(700),
        "balance should be persisted after successful call"
    );
    assert_eq!(
        env.events().all().len(),
        1,
        "exactly one event should be emitted on successful deduct"
    );
}

/// Verifies that even after a failed fee deduction (settlement panics),
/// normal operation can resume with a healthy settlement.
#[test]
fn fee_deduct_recovers_after_settlement_stops_panicking() {
    let env = Env::default();

    // First, try with a panicking settlement — must fail atomically.
    let panicking_addr = env.register(PanickingSettlement, ());
    let caller_addr = env.register(FeeCaller, ());
    let caller_client = FeeCallerClient::new(&env, &caller_addr);

    caller_client.init(&500);
    let fail_result = caller_client.try_deduct(&panicking_addr, &100, &10);
    assert!(
        fail_result.is_err(),
        "expected failure on panicking settlement"
    );
    assert_eq!(
        balance(&env, &caller_addr),
        Some(500),
        "balance must survive a panicking settlement"
    );

    // Now register a healthy settlement and verify deduction works.
    let ok_addr = env.register(OkSettlement, ());
    let remaining = caller_client.deduct(&ok_addr, &200, &11);
    assert_eq!(remaining, 300, "expected remaining after healthy deduction");
    assert_eq!(
        balance(&env, &caller_addr),
        Some(300),
        "balance should now be 300"
    );

    // Only the successful deduction should have emitted an event.
    assert_eq!(
        env.events().all().len(),
        1,
        "only the successful deduct should have emitted an event"
    );
}
