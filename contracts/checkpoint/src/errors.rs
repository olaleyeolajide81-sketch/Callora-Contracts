use soroban_sdk::contracterror;

/// Stable, machine-readable error codes for the Checkpoint contract.
///
/// The numeric discriminants in this enum are part of the contract interface and
/// must remain stable over time. Callers and indexers may branch on these `u32`
/// codes instead of parsing panic strings.
///
/// | Code | Variant              | Meaning                                              |
/// |------|----------------------|------------------------------------------------------|
/// | 1    | NotInitialized       | Contract has not been initialized yet                |
/// | 2    | AlreadyInitialized   | `init` was called more than once                     |
/// | 3    | Unauthorized         | Caller is not authorized for this operation          |
/// | 4    | BatchEmpty           | Batch operation received an empty vector             |
/// | 5    | BatchTooLarge        | Batch operation exceeded the maximum allowed size    |
/// | 6    | CheckpointNotFound   | Requested checkpoint ID does not exist               |
/// | 7    | AmountNegative       | Snapshot balance must not be negative                |
/// | 8    | InvalidPageSize      | Page size must be greater than zero                  |
/// | 9    | Overflow             | Arithmetic overflow detected                         |
/// | 10   | NoAdminTransferPending | No admin transfer is pending                         |
#[contracterror]
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u32)]
pub enum CheckpointError {
    /// Contract has not been initialized yet (code 1).
    NotInitialized = 1,
    /// `init` was called more than once (code 2).
    AlreadyInitialized = 2,
    /// Caller is not authorized for this operation (code 3).
    Unauthorized = 3,
    /// Batch operation received an empty vector (code 4).
    BatchEmpty = 4,
    /// Batch operation exceeded the maximum allowed size (code 5).
    BatchTooLarge = 5,
    /// Requested checkpoint ID does not exist (code 6).
    CheckpointNotFound = 6,
    /// Snapshot balance must not be negative (code 7).
    AmountNegative = 7,
    /// Page size must be greater than zero (code 8).
    InvalidPageSize = 8,
    /// Arithmetic overflow detected (code 9).
    Overflow = 9,
    /// No admin transfer is pending (code 10).
    NoAdminTransferPending = 10,
}
