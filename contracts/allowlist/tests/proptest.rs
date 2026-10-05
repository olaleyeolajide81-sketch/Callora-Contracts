//! Property-based test: whitelist/allowlist invariants (Closes #736).
//!
//! # Invariants tested
//!
//! 1. **Membership**: After `set_allowed_depositor(Some(addr))`,
//!    `is_authorized_depositor(addr)` returns `true`. After
//!    `clear_allowed_depositors()` it returns `false`.
//!
//! 2. **List**: `get_allowed_depositors()` returns the addresses previously
//!    added. After a clear it returns the empty list. Duplicate additions do
//!    not extend the list.
//!
//! 3. **Owner bypass**: The vault owner can always deposit regardless of the
//!    allowlist state.
//!
//! 4. **Authorization check**: When the allowlist is non-empty, a non-owner,
//!    non-allowed depositor cannot deposit. When the allowlist is empty
//!    (no addresses added), there is no restriction.
//!
//! # Generator
//! A deterministic LCG PRNG drives 32 seeded traces (seeds 0..=31) of length
//! 64. Operations include: add, add-duplicate (idempotent), clear, assert
//! membership, assert list, owner-deposit, non-allowed-rejected.
//!
//! # Reproduction
//! On failure the full step trace is printed so the failing seed and operation
//! sequence can be replayed trivially.

extern crate std;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{token, Address, Env, Vec};

use callora_vault::{CalloraVault, CalloraVaultClient};

/// Minimum number of operations per seeded trace.
const TRACE_LENGTH: u32 = 64;

/// Number of deterministic seeds: 0..=SEED_COUNT-1.
const SEED_COUNT: u64 = 32;

/// Pool of depositor addresses the test uses.
const ADDR_POOL_SIZE: usize = 5;

// ---------------------------------------------------------------------------
// Deterministic PRNG
// ---------------------------------------------------------------------------

struct Prng {
    state: u64,
}

impl Prng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(0x9E37_79B9_7F4A_7C15),
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        self.state
    }

    fn gen_range_usize(&mut self, min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        min + (self.next_u64() as usize) % (max - min)
    }

    fn next_bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }
}

// ---------------------------------------------------------------------------
// Trace recording (printed on invariant violation)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct TraceStep {
    index: u32,
    label: &'static str,
    detail: std::string::String,
}

struct Trace {
    seed: u64,
    steps: std::vec::Vec<TraceStep>,
}

impl Trace {
    fn new(seed: u64) -> Self {
        Self {
            seed,
            steps: std::vec::Vec::new(),
        }
    }

    fn push(&mut self, index: u32, label: &'static str, detail: impl Into<std::string::String>) {
        self.steps.push(TraceStep {
            index,
            label,
            detail: detail.into(),
        });
    }

    fn panic_msg(&self, msg: std::string::String) -> ! {
        let mut out = std::format!(
            "INVARIANT VIOLATION: {msg}\nseed={}\n--- trace ---\n",
            self.seed
        );
        for s in &self.steps {
            out.push_str(&std::format!(
                "  [{}] {} — {}\n",
                s.index,
                s.label,
                s.detail
            ));
        }
        panic!("{out}");
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn create_usdc<'a>(
    env: &'a Env,
    admin: &Address,
) -> (Address, token::Client<'a>, token::StellarAssetClient<'a>) {
    let ca = env.register_stellar_asset_contract_v2(admin.clone());
    let addr = ca.address();
    (
        addr.clone(),
        token::Client::new(env, &addr),
        token::StellarAssetClient::new(env, &addr),
    )
}

fn create_vault(env: &Env) -> (Address, CalloraVaultClient<'_>) {
    let addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &addr);
    (addr, client)
}

fn to_std_vec(soroban: Vec<Address>) -> std::vec::Vec<Address> {
    let mut v = std::vec::Vec::with_capacity(soroban.len() as usize);
    for a in soroban.iter() {
        v.push(a);
    }
    v
}

// ---------------------------------------------------------------------------
// Operation kinds (generator alphabet)
// ---------------------------------------------------------------------------

#[repr(u8)]
enum OpKind {
    Add = 0,
    AddDuplicate = 1,
    Clear = 2,
    AssertMembership = 3,
    AssertList = 4,
    OwnerDeposit = 5,
    NonAllowedRejected = 6,
}

const OP_COUNT: u64 = 7;

// ---------------------------------------------------------------------------
// Core property runner
// ---------------------------------------------------------------------------

fn run_property_trace(seed: u64) {
    let env = Env::default();
    env.mock_all_auths();

    let mut trace = Trace::new(seed);
    let mut rng = Prng::new(seed);

    let owner = Address::generate(&env);
    let mut pool: std::vec::Vec<Address> = std::vec::Vec::with_capacity(ADDR_POOL_SIZE);
    for _ in 0..ADDR_POOL_SIZE {
        pool.push(Address::generate(&env));
    }

    let (vault_addr, client) = create_vault(&env);
    let (_usdc_addr, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    // Minimal init so depositors can call deposit.
    usdc_admin.mint(&vault_addr, &1_000_000);
    client.init(
        &owner,
        &_usdc_addr,
        &Some(1_000_000),
        &None,
        &Some(1),
        &None,
        &Some(10_000),
        &None,
    );

    // Fund all participants and approve vault.
    usdc_admin.mint(&owner, &1_000_000);
    usdc_client.approve(&owner, &vault_addr, &i128::MAX, &999_999);
    for addr in &pool {
        usdc_admin.mint(addr, &1_000_000);
        usdc_client.approve(addr, &vault_addr, &i128::MAX, &999_999);
    }

    // Track which addresses have been allowed by the owner.
    let mut allowed: std::vec::Vec<Address> = std::vec::Vec::new();

    for step in 1..=TRACE_LENGTH {
        let op = (rng.next_u64() % OP_COUNT) as u8;

        match op {
            x if x == OpKind::Add as u8 => {
                let idx = rng.gen_range_usize(0, pool.len());
                let addr = pool[idx].clone();

                client.set_allowed_depositor(&owner, &Some(addr.clone()));

                if !allowed.iter().any(|a| *a == addr) {
                    allowed.push(addr.clone());
                }

                assert!(
                    client.is_authorized_depositor(&addr),
                    "just-added address must be authorized"
                );

                trace.push(step, "add", std::format!("addr={addr:?}"));
            }

            x if x == OpKind::AddDuplicate as u8 => {
                if allowed.is_empty() {
                    trace.push(step, "add_duplicate (skip)", "allowlist empty");
                    continue;
                }
                let idx = rng.gen_range_usize(0, allowed.len());
                let addr = allowed[idx].clone();

                client.set_allowed_depositor(&owner, &Some(addr.clone()));

                let list = to_std_vec(client.get_allowed_depositors());
                assert_eq!(
                    list.len(),
                    allowed.len(),
                    "duplicate add must not extend the list"
                );

                trace.push(step, "add_duplicate", std::format!("addr={addr:?}"));
            }

            x if x == OpKind::Clear as u8 => {
                client.clear_allowed_depositors(&owner);
                allowed.clear();

                for addr in &pool {
                    assert!(
                        !client.is_authorized_depositor(addr),
                        "after clear, no pool address is authorized"
                    );
                }
                assert!(
                    client.is_authorized_depositor(&owner),
                    "owner must always be authorized"
                );

                let list = to_std_vec(client.get_allowed_depositors());
                assert!(
                    list.is_empty(),
                    "after clear, get_allowed_depositors returns empty"
                );

                trace.push(step, "clear", "");
            }

            x if x == OpKind::AssertMembership as u8 => {
                if allowed.is_empty() {
                    let idx = rng.gen_range_usize(0, pool.len());
                    let addr = pool[idx].clone();
                    assert!(
                        !client.is_authorized_depositor(&addr),
                        "non-allowed depositor must NOT be authorized"
                    );
                    trace.push(
                        step,
                        "assert_non_member",
                        std::format!("addr={addr:?} (allowlist empty)"),
                    );
                } else {
                    let idx = rng.gen_range_usize(0, allowed.len());
                    let addr = allowed[idx].clone();
                    assert!(
                        client.is_authorized_depositor(&addr),
                        "allowed depositor must be authorized"
                    );
                    trace.push(step, "assert_member", std::format!("addr={addr:?}"));
                }

                assert!(
                    client.is_authorized_depositor(&owner),
                    "owner must always be authorized"
                );

                let non_allowed: std::vec::Vec<&Address> = pool
                    .iter()
                    .filter(|a| !allowed.iter().any(|al| al == *a))
                    .collect();
                if let Some(na) = non_allowed.first() {
                    assert!(
                        !client.is_authorized_depositor(*na),
                        "non-allowed depositor must NOT be authorized"
                    );
                }
            }

            x if x == OpKind::AssertList as u8 => {
                let list = to_std_vec(client.get_allowed_depositors());
                assert_eq!(
                    list.len(),
                    allowed.len(),
                    "list length must match tracked count"
                );
                for a in &list {
                    assert!(allowed.contains(a), "list entry must be in tracked set");
                }
                for a in &allowed {
                    assert!(list.contains(a), "tracked entry must be in list");
                }
                trace.push(step, "assert_list", std::format!("len={}", list.len()));
            }

            x if x == OpKind::OwnerDeposit as u8 => {
                let result = client.try_deposit(&owner, &1);
                assert!(result.is_ok(), "owner must always be able to deposit");
                trace.push(step, "owner_deposit", "ok");
            }

            x if x == OpKind::NonAllowedRejected as u8 => {
                let non_allowed: std::vec::Vec<&Address> = pool
                    .iter()
                    .filter(|a| !allowed.iter().any(|al| al == *a))
                    .collect();
                if let Some(na) = non_allowed.first() {
                    let result = client.try_deposit(na, &1);
                    if allowed.is_empty() {
                        // Empty allowlist = unrestricted.
                        assert!(
                            result.is_ok(),
                            "non-allowed depositor must be accepted when allowlist is empty"
                        );
                        trace.push(
                            step,
                            "non_allowed_deposit (empty list)",
                            std::format!("accepted addr={na:?}"),
                        );
                    } else {
                        assert!(
                            result.is_err(),
                            "non-allowed depositor must be rejected when allowlist is active"
                        );
                        trace.push(
                            step,
                            "non_allowed_deposit (rejected)",
                            std::format!("addr={na:?}"),
                        );
                    }
                } else {
                    trace.push(step, "non_allowed_deposit (skip)", "everyone is allowed");
                }
            }

            _ => unreachable!(),
        }
    }

    // Final assertions.
    let list = to_std_vec(client.get_allowed_depositors());
    if list.len() != allowed.len() {
        trace.panic_msg(std::format!(
            "final list length mismatch: got {} expected {}",
            list.len(),
            allowed.len()
        ));
    }
    for addr in &allowed {
        let auth = client.is_authorized_depositor(addr);
        if !auth {
            trace.panic_msg(std::format!(
                "final: allowed addr {addr:?} is not authorized"
            ));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Run all 32 deterministic seeded traces (seeds 0..=31), each of length 64.
#[test]
fn test_whitelist_invariant_seeded_traces() {
    for seed in 0..SEED_COUNT {
        run_property_trace(seed);
    }
}

/// Edge case: empty allowlist (no addresses added) — all depositors allowed.
#[test]
fn test_whitelist_empty_list_allows_all() {
    let env = Env::default();
    env.mock_all_auths();

    let owner = Address::generate(&env);
    let depositor = Address::generate(&env);

    let (vault_addr, client) = create_vault(&env);
    let (_usdc_addr, usdc_client, usdc_admin) = create_usdc(&env, &owner);

    usdc_admin.mint(&vault_addr, &1_000);
    client.init(
        &owner,
        &_usdc_addr,
        &Some(1_000),
        &None,
        &Some(1),
        &None,
        &Some(1_000),
        &None,
    );
    usdc_admin.mint(&depositor, &1_000);
    usdc_client.approve(&depositor, &vault_addr, &i128::MAX, &999_999);

    assert!(client.get_allowed_depositors().is_empty());
    let result = client.try_deposit(&depositor, &100);
    assert!(
        result.is_ok(),
        "depositor must be allowed when allowlist is empty"
    );
}

/// Edge case: clear on an already-empty allowlist is idempotent.
#[test]
fn test_whitelist_clear_idempotent() {
    let env = Env::default();
    env.mock_all_auths();

    let owner = Address::generate(&env);

    let (vault_addr, client) = create_vault(&env);
    let (_usdc_addr, _, usdc_admin) = create_usdc(&env, &owner);

    usdc_admin.mint(&vault_addr, &1_000);
    client.init(
        &owner,
        &_usdc_addr,
        &Some(1_000),
        &None,
        &Some(1),
        &None,
        &Some(1_000),
        &None,
    );

    client.clear_allowed_depositors(&owner);
    assert!(client.get_allowed_depositors().is_empty());

    client.clear_allowed_depositors(&owner);
    assert!(client.get_allowed_depositors().is_empty());
}
