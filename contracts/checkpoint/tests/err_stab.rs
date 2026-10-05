#![cfg(test)]

use callora_checkpoint::errors::CheckpointError;

#[test]
fn test_checkpoint_error_stability() {
    // Freeze client-facing error code numbers for checkpoint.
    assert_eq!(CheckpointError::NotInitialized as u32, 1);
    assert_eq!(CheckpointError::AlreadyInitialized as u32, 2);
    assert_eq!(CheckpointError::Unauthorized as u32, 3);
    assert_eq!(CheckpointError::BatchEmpty as u32, 4);
    assert_eq!(CheckpointError::BatchTooLarge as u32, 5);
    assert_eq!(CheckpointError::CheckpointNotFound as u32, 6);
    assert_eq!(CheckpointError::AmountNegative as u32, 7);
    assert_eq!(CheckpointError::InvalidPageSize as u32, 8);
    assert_eq!(CheckpointError::Overflow as u32, 9);
    assert_eq!(CheckpointError::NoAdminTransferPending as u32, 10);
}
