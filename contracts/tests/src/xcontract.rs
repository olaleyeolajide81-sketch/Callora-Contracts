//! Cross-contract call safety tests (issue #748).
//!
//! Verifies that when a callee contract reverts/panics mid-call, the failure
//! is surfaced to the caller as an `Err` (via the generated `try_*` client),
//! never a hard host abort, and that Soroban's atomic-invocation guarantee
//! holds end-to-end: no partial state and no partial events survive from
//! either side of the failed call.
//!
//! The existing `vault` -> `settlement` link can't be used to exercise this
//! directly: in native/test builds `vault`'s `settlement::Client` is a no-op
//! stub (see `contracts/vault/src/lib.rs`), so it never actually dispatches a
//! cross-contract call for a unit test to observe. Two minimal contracts are
//! used here instead so the invariant is tested against a real cross-contract
//! invocation rather than the stub.

extern crate std;

use soroban_sdk::{contract, contractimpl, testutils::Events as _, Address, Env, Symbol, Vec};

/// Callee that always reverts. Stands in for any downstream contract
/// (e.g. `settlement`) failing mid-call.
mod panicking {
    use super::*;

    #[contract]
    pub struct AlwaysPanicsCallee;

    #[contractimpl]
    impl AlwaysPanicsCallee {
        /// Always panics; return type only exists to match [`OkCallee::boom`].
        pub fn boom(_env: Env) -> i128 {
            panic!("callee deliberately reverted");
        }
    }
}
pub use panicking::AlwaysPanicsCallee;

/// Callee that succeeds. Used as the control case.
mod ok {
    use super::*;

    #[contract]
    pub struct OkCallee;

    #[contractimpl]
    impl OkCallee {
        pub fn boom(_env: Env) -> i128 {
            42
        }
    }
}
pub use ok::OkCallee;

/// Minimal caller mirroring the `deduct` pattern used throughout this
/// workspace: write local state, invoke a callee, then emit an event only
/// once the callee has returned successfully.
#[contract]
pub struct Caller;

#[contractimpl]
impl Caller {
    pub fn call_and_emit(env: Env, callee: Address) -> i128 {
        env.storage()
            .instance()
            .set(&Symbol::new(&env, "hits"), &1u32);

        let result: i128 = env.invoke_contract(&callee, &Symbol::new(&env, "boom"), Vec::new(&env));

        env.events().publish((Symbol::new(&env, "done"),), ());
        result
    }
}

fn hits(env: &Env, caller_addr: &Address) -> Option<u32> {
    env.as_contract(caller_addr, || {
        env.storage().instance().get(&Symbol::new(env, "hits"))
    })
}

#[test]
fn callee_panic_is_surfaced_as_err_not_host_abort() {
    let env = Env::default();
    let callee_addr = env.register(AlwaysPanicsCallee, ());
    let caller_addr = env.register(Caller, ());
    let caller_client = CallerClient::new(&env, &caller_addr);

    let result = caller_client.try_call_and_emit(&callee_addr);

    assert!(
        result.is_err(),
        "a reverting callee must surface as Err, not abort the test host"
    );
}

#[test]
fn callee_panic_rolls_back_caller_state_and_events() {
    let env = Env::default();
    let callee_addr = env.register(AlwaysPanicsCallee, ());
    let caller_addr = env.register(Caller, ());
    let caller_client = CallerClient::new(&env, &caller_addr);

    let _ = caller_client.try_call_and_emit(&callee_addr);

    assert!(
        env.events().all().is_empty(),
        "no event should survive a reverted cross-contract call"
    );
    assert_eq!(
        hits(&env, &caller_addr),
        None,
        "state written before a reverted cross-call must not persist"
    );
}

#[test]
fn successful_cross_call_persists_state_and_emits_event() {
    let env = Env::default();
    let callee_addr = env.register(OkCallee, ());
    let caller_addr = env.register(Caller, ());
    let caller_client = CallerClient::new(&env, &caller_addr);

    let returned = caller_client.call_and_emit(&callee_addr);

    assert_eq!(returned, 42);
    assert_eq!(env.events().all().len(), 1);
    assert_eq!(hits(&env, &caller_addr), Some(1));
}
