#![cfg(test)]

use callora_vault::{CalloraVault, CalloraVaultClient};
use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{token, Address, Env, IntoVal, Symbol, TryFromVal};

fn setup(
    env: &Env,
) -> (
    CalloraVaultClient<'_>,
    Address,
    Address,
    Address,
    Address,
    Address,
) {
    env.mock_all_auths();
    let owner = Address::generate(env);
    let admin = Address::generate(env);
    let vault_addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(env, &vault_addr);

    let usdc_addr = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let usdc_admin = token::StellarAssetClient::new(env, &usdc_addr);

    let other_addr = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let other_admin = token::StellarAssetClient::new(env, &other_addr);

    client.init(
        &owner,
        &usdc_addr,
        &Some(1000i128),
        &Some(admin.clone()),
        &Some(1),
        &None,
        &Some(1_000_000),
        &Some(env.register(CalloraVault, ())),
    );
    client.set_admin(&owner, &admin);

    usdc_admin.mint(&vault_addr, &5000i128);
    other_admin.mint(&vault_addr, &3000i128);

    (client, owner, admin, usdc_addr, other_addr, vault_addr)
}

#[test]
fn test_rescue_non_usdc_full_balance() {
    let env = Env::default();
    let (client, _owner, admin, _usdc_addr, other_addr, _vault_addr) = setup(&env);
    let recipient = Address::generate(&env);

    client.admin_rescue(&admin, &other_addr, &recipient, &3000i128);

    let token_client = token::Client::new(&env, &other_addr);
    assert_eq!(token_client.balance(&recipient), 3000i128);
}

#[test]
fn test_rescue_non_usdc_partial_amount() {
    let env = Env::default();
    let (client, _owner, admin, _usdc_addr, other_addr, _vault_addr) = setup(&env);
    let recipient = Address::generate(&env);

    client.admin_rescue(&admin, &other_addr, &recipient, &1000i128);

    let token_client = token::Client::new(&env, &other_addr);
    assert_eq!(token_client.balance(&recipient), 1000i128);
}

#[test]
fn test_rescue_usdc_surplus_only() {
    let env = Env::default();
    let (client, _owner, admin, usdc_addr, _other_addr, _vault_addr) = setup(&env);
    let recipient = Address::generate(&env);

    client.admin_rescue(&admin, &usdc_addr, &recipient, &4000i128);

    let usdc = token::Client::new(&env, &usdc_addr);
    assert_eq!(usdc.balance(&recipient), 4000i128);
}

#[test]
fn test_rescue_usdc_protected_balance_enforced() {
    let env = Env::default();
    let (client, _owner, admin, usdc_addr, _other_addr, _vault_addr) = setup(&env);
    let recipient = Address::generate(&env);

    let result = client.try_admin_rescue(&admin, &usdc_addr, &recipient, &5000i128);
    assert!(result.is_err());
}

#[test]
fn test_rescue_zero_amount_rejected() {
    let env = Env::default();
    let (client, _owner, admin, _usdc_addr, other_addr, _vault_addr) = setup(&env);
    let recipient = Address::generate(&env);

    let result = client.try_admin_rescue(&admin, &other_addr, &recipient, &0i128);
    assert!(result.is_err());
}

#[test]
fn test_rescue_negative_amount_rejected() {
    let env = Env::default();
    let (client, _owner, admin, _usdc_addr, other_addr, _vault_addr) = setup(&env);
    let recipient = Address::generate(&env);

    let result = client.try_admin_rescue(&admin, &other_addr, &recipient, &-100i128);
    assert!(result.is_err());
}

#[test]
fn test_rescue_insufficient_balance_rejected() {
    let env = Env::default();
    let (client, _owner, admin, _usdc_addr, other_addr, _vault_addr) = setup(&env);
    let recipient = Address::generate(&env);

    let result = client.try_admin_rescue(&admin, &other_addr, &recipient, &5000i128);
    assert!(result.is_err());
}

#[test]
fn test_rescue_emits_event() {
    let env = Env::default();
    let (client, _owner, admin, _usdc_addr, other_addr, _vault_addr) = setup(&env);
    let recipient = Address::generate(&env);

    client.admin_rescue(&admin, &other_addr, &recipient, &500i128);

    let events = env.events().all();
    let found = events.iter().any(|ev| {
        !ev.1.is_empty()
            && Symbol::try_from_val(&env, &ev.1.get(0).unwrap())
                .map(|s| s == Symbol::new(&env, "rescue_funds"))
                .unwrap_or(false)
    });
    assert!(found, "Expected rescue_funds event to be emitted");
}

#[test]
fn test_rescue_checked_sub_overflow_safe() {
    let env = Env::default();
    let (client, _owner, admin, usdc_addr, _other_addr, vault_addr) = setup(&env);
    let recipient = Address::generate(&env);
    let usdc = token::Client::new(&env, &usdc_addr);

    usdc.transfer(&vault_addr, &recipient, &5000i128);

    let result = client.try_admin_rescue(&admin, &usdc_addr, &recipient, &1i128);
    assert!(result.is_err());
}

#[test]
fn test_rescue_unauthorized_caller_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let owner = Address::generate(&env);
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let vault_addr = env.register(CalloraVault, ());
    let client = CalloraVaultClient::new(&env, &vault_addr);

    let usdc_addr = env
        .register_stellar_asset_contract_v2(owner.clone())
        .address();
    let usdc_admin = token::StellarAssetClient::new(&env, &usdc_addr);

    client.init(
        &owner,
        &usdc_addr,
        &Some(1000i128),
        &Some(admin.clone()),
        &Some(1),
        &None,
        &Some(1_000_000),
        &Some(env.register(CalloraVault, ())),
    );
    client.set_admin(&owner, &admin);

    usdc_admin.mint(&vault_addr, &5000i128);

    let recipient = Address::generate(&env);
    let result = client.try_admin_rescue(&attacker, &usdc_addr, &recipient, &100i128);
    assert!(result.is_err());
}
