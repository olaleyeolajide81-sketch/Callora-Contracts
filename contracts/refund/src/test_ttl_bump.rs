#![cfg(test)]

//! Tests for TTL bump on hot read paths in the refund contract.
//!
//! Hot read paths (public view functions) now bump instance and/or persistent
//! storage TTL so that frequently-queried contracts do not archive due to
//! infrequent writes.

use crate::{
    RefundContract, RefundContractClient, RefundError, RefundStatus, INSTANCE_BUMP_AMOUNT,
    INSTANCE_BUMP_THRESHOLD, PERSISTENT_BUMP_AMOUNT, PERSISTENT_BUMP_THRESHOLD,
};
use soroban_sdk::testutils::{storage::Instance, storage::Persistent, Address as _, Ledger};
use soroban_sdk::{Address, Env, Symbol};

fn setup() -> (Env, Address, RefundContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(RefundContract, ());
    let client = RefundContractClient::new(&env, &contract_id);
    client.init(&admin, &250, &100);
    (env, admin, client)
}

#[test]
fn test_get_admin_bumps_instance_ttl() {
    let (env, admin, client) = setup();

    // Advance sequence number to reduce instance TTL below threshold (but not expired).
    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let ttl_before = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert!(ttl_before < INSTANCE_BUMP_THRESHOLD);

    let res = client.get_admin();
    assert_eq!(res, admin);

    let ttl_after = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_config_bumps_instance_ttl() {
    let (env, _admin, client) = setup();

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let config = client.get_config();
    assert_eq!(config.fee_bps, 250);

    let ttl_after = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_total_refunds_bumps_instance_ttl() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    // Create and process a refund to have some total
    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));
    client.approve_refund(&admin, &request_id);
    client.process_refund(&admin, &request_id);

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let total = client.get_total_refunds();
    assert_eq!(total, 500);

    let ttl_after = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_refund_counter_bumps_instance_ttl() {
    let (env, _admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let counter = client.get_refund_counter();
    assert_eq!(counter, 1);

    let ttl_after = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_refund_request_bumps_persistent_ttl() {
    let (env, _admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));

    let key = crate::types::StorageKey::PendingRefund(request_id);

    // Advance sequence number to decrease persistent TTL below threshold
    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let ttl_before = env.as_contract(&env.current_contract_address(), || {
        env.storage().persistent().get_ttl(&key)
    });
    assert!(ttl_before < PERSISTENT_BUMP_THRESHOLD);

    let request = client.get_refund_request(&request_id);
    assert_eq!(request.id, request_id);

    let ttl_after = env.as_contract(&env.current_contract_address(), || {
        env.storage().persistent().get_ttl(&key)
    });
    assert_eq!(ttl_after, PERSISTENT_BUMP_AMOUNT);

    // Instance TTL should also be bumped
    let inst_ttl = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert_eq!(inst_ttl, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_get_refund_request_not_found() {
    let (env, _admin, client) = setup();

    let result = client.try_get_refund_request(&999);
    assert_eq!(result.unwrap_err().unwrap(), RefundError::NotFound);
}

#[test]
fn test_multiple_refund_requests_bump_ttl() {
    let (env, admin, client) = setup();
    let requester1 = Address::generate(&env);
    let requester2 = Address::generate(&env);
    let token = Address::generate(&env);

    let id1 = client.request_refund(&requester1, &token, &500, &Symbol::new(&env, "test1"));
    let id2 = client.request_refund(&requester2, &token, &300, &Symbol::new(&env, "test2"));

    let key1 = crate::types::StorageKey::PendingRefund(id1);
    let key2 = crate::types::StorageKey::PendingRefund(id2);

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + PERSISTENT_BUMP_AMOUNT - PERSISTENT_BUMP_THRESHOLD + 10);

    let _ = client.get_refund_request(&id1);
    let _ = client.get_refund_request(&id2);

    let ttl1 = env.as_contract(&env.current_contract_address(), || {
        env.storage().persistent().get_ttl(&key1)
    });
    let ttl2 = env.as_contract(&env.current_contract_address(), || {
        env.storage().persistent().get_ttl(&key2)
    });
    assert_eq!(ttl1, PERSISTENT_BUMP_AMOUNT);
    assert_eq!(ttl2, PERSISTENT_BUMP_AMOUNT);

    let inst_ttl = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert_eq!(inst_ttl, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_request_refund_bumps_instance_ttl() {
    let (env, _admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let ttl_before = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert!(ttl_before < INSTANCE_BUMP_THRESHOLD);

    let _ = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));

    let ttl_after = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_approve_refund_bumps_instance_ttl() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let ttl_before = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert!(ttl_before < INSTANCE_BUMP_THRESHOLD);

    client.approve_refund(&admin, &request_id);

    let ttl_after = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_reject_refund_bumps_instance_ttl() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let ttl_before = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert!(ttl_before < INSTANCE_BUMP_THRESHOLD);

    client.reject_refund(&admin, &request_id);

    let ttl_after = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_process_refund_bumps_instance_ttl() {
    let (env, admin, client) = setup();
    let requester = Address::generate(&env);
    let token = Address::generate(&env);

    let request_id = client.request_refund(&requester, &token, &500, &Symbol::new(&env, "test"));
    client.approve_refund(&admin, &request_id);

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let ttl_before = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert!(ttl_before < INSTANCE_BUMP_THRESHOLD);

    client.process_refund(&admin, &request_id);

    let ttl_after = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}

#[test]
fn test_update_config_bumps_instance_ttl() {
    let (env, admin, client) = setup();

    let seq = env.ledger().sequence();
    env.ledger()
        .set_sequence_number(seq + INSTANCE_BUMP_AMOUNT - INSTANCE_BUMP_THRESHOLD + 10);

    let ttl_before = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert!(ttl_before < INSTANCE_BUMP_THRESHOLD);

    client.update_config(&admin, &500, &200);

    let ttl_after = env.as_contract(&env.current_contract_address(), || {
        env.storage().instance().get_ttl()
    });
    assert_eq!(ttl_after, INSTANCE_BUMP_AMOUNT);
}
