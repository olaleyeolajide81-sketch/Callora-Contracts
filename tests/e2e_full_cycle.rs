//! # End-to-end full-cycle smoke test
//!
//! Wires `callora-vault`, `callora-settlement`, and `callora-revenue-pool`
//! together in a single [`soroban_sdk::Env`] and walks the complete
//! production fund-cycle, asserting balances at every stage.
//!
//! ## Why this test exists
//!
//! Unit tests cover each contract in isolation, and an existing
//! vault↔settlement integration test covers that one pairing. Neither catches
//! a refactor that silently breaks the *full* pipeline an operator actually
//! runs in production. This test is intended to be the project's smoke test:
//! if it's red, the platform is broken end-to-end, full stop.
//!
//! ## The real fund topology (read this before extending the test)
//!
//! The issue that prompted this test described the pipeline as a single
//! chain: `vault → settlement → developer withdraw → revenue_pool sweep →
//! admin batch_distribute`. Having read the actual contract sources, **that
//! chain does not exist on-chain.** `callora-settlement` and
//! `callora-revenue-pool` are independent contracts with no call path
//! between them — there is no "sweep" function anywhere in this workspace.
//! The real topology is two **parallel** destinations for vault-deducted
//! funds:
//!
//! ```text
//!                    ┌──────────────────────────────────────────┐
//!                    │                  vault                   │
//!                    │  deposit · deduct/batch_deduct · withdraw │
//!                    │  · distribute (admin sweep of surplus)    │
//!                    └───────────────┬────────────────┬──────────┘
//!                                    │                │
//!                  to_pool=true      │                │ distribute(to=revenue_pool)
//!            (all vault deducts)     │                │ (admin-initiated, separate path)
//!                                    ▼                ▼
//!                       ┌─────────────────────┐  ┌───────────────────────┐
//!                       │  callora-settlement  │  │ callora-revenue-pool  │
//!                       │  global_pool /       │  │ funded by ANY USDC    │
//!                       │  developer_balances   │  │ transfer in; admin    │
//!                       │                      │  │ calls distribute /    │
//!                       │  developer calls     │  │ batch_distribute      │
//!                       │  withdraw_developer_  │  │ to pay developers     │
//!                       │  balance() directly   │  └───────────────────────┘
//!                       └─────────────────────┘
//! ```
//!
//! This test therefore exercises **both** paths in one scenario so the
//! conservation assertion is meaningful across all three contracts:
//!
//! 1. Vault deposit.
//! 2. Many deducts (single `deduct` + `batch_deduct`), always routed to
//!    settlement's global pool (vault hard-codes `to_pool=true`).
//! 3. Settlement admin credits a developer's per-developer balance directly
//!    (`receive_payment(to_pool=false)`) — modeling an off-chain-priced
//!    credit distinct from the vault's pooled deducts — then that developer
//!    calls `withdraw_developer_balance` to pull USDC out of settlement.
//! 4. Vault admin sweeps on-ledger surplus into the revenue pool via
//!    `vault.distribute`, then the revenue pool admin pays a second
//!    developer via `batch_distribute`.
//! 5. Pause-stage edge cases on both vault and revenue_pool.
//! 6. A deliberately-oversized `batch_distribute` to confirm atomic,
//!    all-or-nothing failure with zero partial transfers.
//! 7. A final conservation assertion across every wallet and contract this
//!    test touched.
//!
//! ## Conservation invariant
//!
//! At every checkpoint:
//!
//! ```text
//! vault.balance()
//!   + settlement.global_pool.total_balance
//!   + sum(settlement developer balances for all devs touched)
//!   + revenue_pool.balance()
//!   + sum(on-ledger USDC balances of every wallet that started with 0)
//!   == INITIAL_MINT
//! ```
//!
//! `owner`'s on-ledger wallet balance is included on the right-hand side
//! implicitly by tracking deltas from `INITIAL_MINT`, since `owner` is the
//! address that received the initial mint and pays for deposits.
//!
//! Run with:
//! ```text
//! cargo test --workspace e2e_full_cycle
//! ```

// Pull in the shared setup helper from `scripts/`. Soroban integration tests
// under `tests/` are each compiled as an independent crate, so a plain `mod`
// statement can't reach outside `tests/`; `#[path]` makes the helper file a
// module of *this* binary without needing it published from any contract
// crate's `lib.rs`.
#[path = "../scripts/e2e_setup.rs"]
mod e2e_setup;

use e2e_setup::{setup, Harness, INITIAL_MINT};
use soroban_sdk::{vec, Address, Env};

/// Sum of a [`Harness`]'s settlement-side holdings: the global pool plus
/// every developer balance tracked in settlement. Pulled into a helper so
/// every checkpoint computes the conservation invariant identically.
fn settlement_total(h: &Harness, devs: &[soroban_sdk::Address]) -> i128 {
    let pool = h.settlement.get_global_pool().total_balance;
    let dev_sum: i128 = devs
        .iter()
        .map(|d| h.settlement.get_developer_balance(d, &h.usdc_id))
        .sum();
    pool + dev_sum
}

/// Asserts the platform-wide conservation invariant documented in this
/// file's module doc: every stroop minted at setup is accounted for across
/// the vault, settlement, revenue_pool, and every wallet this test touched.
///
/// `extra_wallets` should list every address (besides `owner`) that might
/// hold USDC at the time of the check — e.g. developer wallets after a
/// withdraw or distribute.
fn assert_conserved(
    h: &Harness,
    devs: &[soroban_sdk::Address],
    extra_wallets: &[soroban_sdk::Address],
) {
    let vault_bal = h.vault.balance();
    let settlement_bal = settlement_total(h, devs);
    let revenue_pool_bal = h.revenue_pool.balance();
    let owner_wallet = h.usdc.balance(&h.owner);
    let wallets_sum: i128 = extra_wallets.iter().map(|w| h.usdc.balance(w)).sum();

    let total = vault_bal + settlement_bal + revenue_pool_bal + owner_wallet + wallets_sum;
    assert_eq!(
        total, INITIAL_MINT,
        "conservation violated: vault={vault_bal} settlement={settlement_bal} \
         revenue_pool={revenue_pool_bal} owner_wallet={owner_wallet} \
         other_wallets={wallets_sum} (expected total {INITIAL_MINT})"
    );
}

/// Full production-cycle smoke test: deposit → many deducts → settlement
/// crediting (pool + per-developer) → developer withdraw → revenue_pool
/// funding → admin batch_distribute, with a conservation check at every
/// stage. See the module-level doc comment for the fund-topology diagram
/// this test is built around.
#[test]
fn e2e_full_cycle() {
    let env = Env::default();
    let h: Harness = setup(&env);
    let devs = [h.dev_a.clone(), h.dev_b.clone()];

    // Sanity: everything starts at zero except owner's wallet.
    assert_conserved(&h, &devs, &[h.dev_a.clone(), h.dev_b.clone()]);
    assert_eq!(h.usdc.balance(&h.owner), INITIAL_MINT);

    // ------------------------------------------------------------------
    // Stage 1 — Deposit
    // ------------------------------------------------------------------
    let deposit_amount: i128 = 600_000_000;
    h.vault.deposit(&h.owner, &deposit_amount);

    assert_eq!(h.vault.balance(), deposit_amount);
    assert_eq!(h.usdc.balance(&h.owner), INITIAL_MINT - deposit_amount);
    assert_conserved(&h, &devs, &[h.dev_a.clone(), h.dev_b.clone()]);

    // ------------------------------------------------------------------
    // Stage 2 — Many deducts (single + batch), all routed to settlement's
    // global pool, since the vault hard-codes `to_pool=true` for every
    // deduct it initiates.
    // ------------------------------------------------------------------
    let single_deduct_amount: i128 = 10_000_000;
    h.vault.deduct(&h.backend, &single_deduct_amount, &1u64);

    let batch_items = vec![
        &env,
        (5_000_000, 2u64),
        (7_500_000, 3u64),
        (2_500_000, 0u64), // no idempotency tracking for this leg
    ];
    let batch_total: i128 = batch_items.iter().map(|(a, _)| a).sum();
    h.vault.batch_deduct(&h.backend, &batch_items);

    let total_deducted = single_deduct_amount + batch_total;
    assert_eq!(h.vault.balance(), deposit_amount - total_deducted);
    assert_eq!(
        h.settlement.get_global_pool().total_balance,
        total_deducted,
        "every vault-initiated deduct must land in settlement's global pool"
    );
    assert_eq!(
        h.usdc.balance(&h.settlement_id),
        total_deducted,
        "settlement must hold the on-ledger USDC the vault transferred"
    );
    assert_conserved(&h, &devs, &[h.dev_a.clone(), h.dev_b.clone()]);

    // Re-using a request_id must be rejected (idempotency), and must not
    // move any funds — re-assert conservation after the rejected attempt.
    let dup_result = h.vault.try_deduct(&h.backend, &single_deduct_amount, &1u64);
    assert!(dup_result.is_err(), "duplicate request_id must be rejected");
    assert_eq!(h.vault.balance(), deposit_amount - total_deducted);
    assert_conserved(&h, &devs, &[h.dev_a.clone(), h.dev_b.clone()]);

    // ------------------------------------------------------------------
    // Stage 3 — Settlement → developer path.
    //
    // The vault only ever credits the global pool, so to exercise the
    // per-developer ledger + direct-withdraw path we model an
    // admin-initiated credit, exactly as `receive_payment`'s interface
    // permits (caller may be "the registered vault OR admin").
    // ------------------------------------------------------------------
    let dev_a_credit: i128 = 4_000_000;
    h.settlement.receive_payment(
        &h.owner,
        &dev_a_credit,
        &false,
        &Some(h.dev_a.clone()),
        &h.usdc_id,
        &1u32,
    );

    assert_eq!(
        h.settlement.get_developer_balance(&h.dev_a, &h.usdc_id),
        dev_a_credit
    );
    assert_eq!(
        h.settlement.get_global_pool().total_balance,
        total_deducted,
        "crediting a developer must not touch the global pool"
    );
    assert_conserved(&h, &devs, &[h.dev_a.clone(), h.dev_b.clone()]);

    // Developer withdraws their settlement balance directly. This requires
    // settlement to hold configured USDC token + actual on-ledger funds —
    // both are satisfied here since the vault already transferred USDC to
    // settlement in stage 2, and `set_usdc_token` is required first.
    h.settlement.set_usdc_token(&h.owner, &h.usdc_id);
    let dev_a_withdraw: i128 = 1_500_000;
    h.settlement
        .withdraw_developer_balance(&h.dev_a, &dev_a_withdraw, &None::<Address>);

    assert_eq!(
        h.settlement.get_developer_balance(&h.dev_a, &h.usdc_id),
        dev_a_credit - dev_a_withdraw
    );
    assert_eq!(h.usdc.balance(&h.dev_a), dev_a_withdraw);
    assert_conserved(&h, &devs, &[h.dev_a.clone(), h.dev_b.clone()]);

    // ------------------------------------------------------------------
    // Stage 4 — Vault → revenue_pool sweep, then admin batch_distribute.
    //
    // `vault.distribute` moves *untracked on-ledger surplus* (it checks the
    // real token balance, not `meta.balance`), so top up the vault's
    // on-ledger USDC directly to give the admin something real to sweep,
    // mirroring how surplus might accumulate from rounding or direct
    // transfers in production.
    // ------------------------------------------------------------------
    let surplus: i128 = 3_000_000;
    h.usdc.transfer(&h.owner, &h.vault_id, &surplus);

    let sweep_amount: i128 = 3_000_000;
    h.vault
        .distribute(&h.owner, &h.revenue_pool_id, &sweep_amount);

    assert_eq!(h.revenue_pool.balance(), sweep_amount);
    assert_conserved(&h, &devs, &[h.dev_a.clone(), h.dev_b.clone()]);

    // Admin pays dev_b out of the revenue pool via batch_distribute.
    let dev_b_payment: i128 = 2_000_000;
    h.revenue_pool
        .batch_distribute(&h.owner, &vec![&env, (h.dev_b.clone(), dev_b_payment)]);

    assert_eq!(h.usdc.balance(&h.dev_b), dev_b_payment);
    assert_eq!(h.revenue_pool.balance(), sweep_amount - dev_b_payment);
    assert_conserved(&h, &devs, &[h.dev_a.clone(), h.dev_b.clone()]);

    // ------------------------------------------------------------------
    // Stage 5 — Pause edge cases.
    // ------------------------------------------------------------------

    // 5a. Paused vault blocks deposit and deduct, but still allows owner
    //     withdraw and admin distribute (per the vault's documented pause
    //     policy — see `CalloraVault`'s module doc).
    h.vault.pause(&h.owner);
    assert!(h.vault.is_paused());

    let blocked_deposit = h.vault.try_deposit(&h.owner, &1_000);
    assert!(
        blocked_deposit.is_err(),
        "deposit must be blocked while paused"
    );

    let blocked_deduct = h.vault.try_deduct(&h.backend, &1_000, &0u64);
    assert!(
        blocked_deduct.is_err(),
        "deduct must be blocked while paused"
    );

    // Owner withdraw is explicitly allowed while paused (emergency recovery).
    let pre_pause_vault_balance = h.vault.balance();
    let withdraw_amount: i128 = 1_000_000;
    h.vault.withdraw(&withdraw_amount);
    assert_eq!(h.vault.balance(), pre_pause_vault_balance - withdraw_amount);

    h.vault.unpause(&h.owner);
    assert!(!h.vault.is_paused());
    assert_conserved(&h, &devs, &[h.dev_a.clone(), h.dev_b.clone()]);

    // 5b. Paused revenue_pool blocks distribute / batch_distribute.
    h.revenue_pool.pause(&h.owner);
    assert!(h.revenue_pool.is_paused());

    let blocked_distribute = h.revenue_pool.try_distribute(&h.owner, &h.dev_b, &1);
    assert!(
        blocked_distribute.is_err(),
        "distribute must be blocked while revenue_pool is paused"
    );
    let blocked_batch = h
        .revenue_pool
        .try_batch_distribute(&h.owner, &vec![&env, (h.dev_b.clone(), 1)]);
    assert!(
        blocked_batch.is_err(),
        "batch_distribute must be blocked while revenue_pool is paused"
    );

    h.revenue_pool.unpause(&h.owner);
    assert!(!h.revenue_pool.is_paused());
    assert_conserved(&h, &devs, &[h.dev_a.clone(), h.dev_b.clone()]);

    // ------------------------------------------------------------------
    // Stage 6 — Partial batch_distribute failure must be all-or-nothing.
    //
    // Request far more than the revenue pool currently holds; the call
    // must revert with NO transfers applied (not even the legs that would
    // individually have succeeded).
    // ------------------------------------------------------------------
    let pre_failure_pool_balance = h.revenue_pool.balance();
    let pre_failure_dev_a_wallet = h.usdc.balance(&h.dev_a);
    let pre_failure_dev_b_wallet = h.usdc.balance(&h.dev_b);

    let oversized_batch = vec![
        &env,
        (h.dev_a.clone(), pre_failure_pool_balance), // alone would succeed
        (h.dev_b.clone(), pre_failure_pool_balance), // pushes total over balance
    ];
    let failure = h
        .revenue_pool
        .try_batch_distribute(&h.owner, &oversized_batch);
    assert!(
        failure.is_err(),
        "batch_distribute must reject a batch whose total exceeds the pool's balance"
    );

    // Confirm zero partial effect: neither leg moved, pool balance unchanged.
    assert_eq!(h.revenue_pool.balance(), pre_failure_pool_balance);
    assert_eq!(h.usdc.balance(&h.dev_a), pre_failure_dev_a_wallet);
    assert_eq!(h.usdc.balance(&h.dev_b), pre_failure_dev_b_wallet);
    assert_conserved(&h, &devs, &[h.dev_a.clone(), h.dev_b.clone()]);

    // ------------------------------------------------------------------
    // Stage 7 — Final conservation assertion.
    //
    // Restated explicitly (rather than only via the helper) so a reviewer
    // can see the literal invariant from the issue's acceptance criteria
    // without having to trace into `assert_conserved`.
    // ------------------------------------------------------------------
    let final_vault = h.vault.balance();
    let final_settlement = settlement_total(&h, &devs);
    let final_revenue_pool = h.revenue_pool.balance();
    let final_owner_wallet = h.usdc.balance(&h.owner);
    let final_dev_a_wallet = h.usdc.balance(&h.dev_a);
    let final_dev_b_wallet = h.usdc.balance(&h.dev_b);

    assert_eq!(
        final_vault
            + final_settlement
            + final_revenue_pool
            + final_owner_wallet
            + final_dev_a_wallet
            + final_dev_b_wallet,
        INITIAL_MINT,
        "final conservation check failed: vault tracked balance + settlement \
         pool/developer balances + revenue_pool balance + every wallet balance \
         must equal total USDC minted at setup"
    );
}
