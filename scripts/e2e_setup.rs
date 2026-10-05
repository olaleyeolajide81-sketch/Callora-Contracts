//! Shared setup boilerplate for the end-to-end full-cycle test
//! (`tests/e2e_full_cycle.rs`).
//!
//! This module deploys all four contracts the platform needs for a single
//! production cycle — a mock USDC token, `callora-vault`, `callora-settlement`,
//! and `callora-revenue-pool` — wires them together exactly as an operator
//! would in production (per `docs/CONTRACT_ADDRESS_CONFIGURATION.md`), and
//! returns a [`Harness`] bundling every client and identity the test needs.
//!
//! Keeping this in `scripts/` (rather than inlining it in the test file)
//! means future E2E or integration tests can reuse the same wiring instead of
//! re-deriving it, and a reviewer only has to read the wiring logic once.
//!
//! NOTE: `tests/` binaries cannot `mod`-include arbitrary paths outside the
//! crate without `#[path]`. This file is wired into `tests/e2e_full_cycle.rs`
//! via `#[path = "../scripts/e2e_setup.rs"] mod e2e_setup;` (both `tests/`
//! and `scripts/` are siblings under the workspace root, hence one `../`) —
//! see the test file header for details. If your project structure differs,
//! adjust that `#[path]` attribute to point here.

use callora_revenue_pool::RevenuePoolClient;
use callora_settlement::CalloraSettlementClient;
use callora_vault::CalloraVaultClient;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{Address, Env};

/// Every contract client, token client, and test identity the E2E suite needs.
///
/// Bundling these in one struct (rather than returning a long tuple) keeps
/// `tests/e2e_full_cycle.rs` readable — each stage of the cycle reaches for
/// `h.vault`, `h.settlement`, `h.revenue_pool`, etc. by name.
pub struct Harness<'a> {
    pub env: Env,

    // Token (mock USDC) — `StellarAssetClient` mints, `TokenClient` reads
    // balances / does ordinary transfers exactly like real USDC would.
    pub usdc_admin_client: StellarAssetClient<'a>,
    pub usdc: TokenClient<'a>,
    pub usdc_id: Address,

    // Contract clients.
    pub vault: CalloraVaultClient<'a>,
    pub settlement: CalloraSettlementClient<'a>,
    pub revenue_pool: RevenuePoolClient<'a>,

    // Contract addresses (handy for cross-contract wiring / assertions
    // without re-deriving them from the clients each time).
    pub vault_id: Address,
    pub settlement_id: Address,
    pub revenue_pool_id: Address,

    // Identities.
    /// Vault owner; also used as the shared admin for settlement and
    /// revenue_pool to keep the harness simple — production deployments
    /// may use distinct admins per contract.
    pub owner: Address,
    /// Backend address authorized to call `deduct` / `batch_deduct` on the
    /// vault (set via `set_authorized_caller`).
    pub backend: Address,
    /// Two developer wallets used to exercise both fund-destination paths:
    /// `dev_a` receives funds via the settlement contract's per-developer
    /// ledger and withdraws them directly; `dev_b` receives funds via the
    /// revenue_pool's `distribute` / `batch_distribute`.
    pub dev_a: Address,
    pub dev_b: Address,
}

/// Total USDC (in stroops) minted to `owner` and deposited into the vault
/// at setup time. Chosen large enough to comfortably cover every deduct,
/// credit, and distribution exercised in the full-cycle scenario.
pub const INITIAL_MINT: i128 = 1_000_000_000;

/// Deploys the mock USDC token plus all three Callora contracts, wires them
/// together (vault → settlement, vault → revenue_pool, settlement → vault,
/// revenue_pool → usdc), mints `INITIAL_MINT` USDC to `owner`, and returns a
/// ready-to-use [`Harness`].
///
/// # Wiring performed
/// 1. Deploy mock USDC (Stellar Asset Contract test token).
/// 2. Deploy `vault`, `settlement`, `revenue_pool`.
/// 3. `vault.init(...)` with `owner`, the mock USDC address, and `revenue_pool`
///    recorded as the (informational) revenue pool address.
/// 4. `vault.set_authorized_caller(backend)` so the backend identity can call
///    `deduct` / `batch_deduct` in the test without using the owner key.
/// 5. `vault.set_settlement(settlement_id)` — required before any deduct call
///    will succeed (`VaultError::SettlementNotSet` otherwise).
/// 6. `settlement.init(owner, vault_id)` — registers the vault as the only
///    non-admin caller permitted to call `receive_payment`.
/// 7. `revenue_pool.init(owner, usdc_id)`.
/// 8. Mint `INITIAL_MINT` USDC to `owner` on-ledger (vault `deposit` requires
///    the caller to already hold the USDC being deposited).
///
/// All calls use `env.mock_all_auths()`, so every `require_auth()` in the
/// contracts is satisfied without constructing real signatures — standard
/// practice for native Soroban SDK tests.
pub fn setup<'a>(env: &Env) -> Harness<'a> {
    env.mock_all_auths();

    let owner = Address::generate(env);
    let backend = Address::generate(env);
    let dev_a = Address::generate(env);
    let dev_b = Address::generate(env);

    // ---- Mock USDC -------------------------------------------------------
    let usdc_id = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let usdc_admin_client = StellarAssetClient::new(env, &usdc_id);
    let usdc = TokenClient::new(env, &usdc_id);

    // ---- Contracts -----------------------------------------------------
    // `register` takes a contract type implementing `Default`/unit-struct
    // construction (all three `#[contract]` structs here are field-less)
    // plus a constructor-args tuple; these contracts have no `__constructor`,
    // so the second argument is `()`.
    let vault_id = env.register(callora_vault::CalloraVault, ());
    let vault = CalloraVaultClient::new(env, &vault_id);

    let settlement_id = env.register(callora_settlement::CalloraSettlement, ());
    let settlement = CalloraSettlementClient::new(env, &settlement_id);

    let revenue_pool_id = env.register(callora_revenue_pool::RevenuePool, ());
    let revenue_pool = RevenuePoolClient::new(env, &revenue_pool_id);

    // ---- Mint working capital before vault init (init can verify on-ledger
    // balance when initial_balance > 0; we pass None and deposit explicitly
    // in the test instead, so minting can happen in either order). ----------
    usdc_admin_client.mint(&owner, &INITIAL_MINT);

    // ---- Initialize vault --------------------------------------------------
    vault.init(
        &owner,
        &usdc_id,
        &Some(0i128),                   // initial_balance
        &Some(backend.clone()),         // authorized_caller
        &Some(1i128),                   // min_deposit
        &Some(revenue_pool_id.clone()), // revenue_pool
        &Some(i128::MAX),               // max_deduct
        &Some(settlement_id.clone()),   // settlement
    );

    // ---- Initialize settlement ---------------------------------------------
    settlement.init(&owner, &vault_id);

    // ---- Initialize revenue pool --------------------------------------------
    revenue_pool.init(&owner, &usdc_id);

    Harness {
        env: env.clone(),
        usdc_admin_client,
        usdc,
        usdc_id,
        vault,
        settlement,
        revenue_pool,
        vault_id,
        settlement_id,
        revenue_pool_id,
        owner,
        backend,
        dev_a,
        dev_b,
    }
}
