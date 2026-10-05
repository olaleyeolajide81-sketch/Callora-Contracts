use soroban_sdk::contracterror;

/// Errors that can be returned by the refund contract.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RefundError {
    /// Contract has already been initialized.
    AlreadyInitialized = 1,
    /// Contract has not been initialized.
    NotInitialized = 2,
    /// Caller is not authorized to perform this action.
    Unauthorized = 3,
    /// Requested refund not found.
    NotFound = 4,
    /// Fee basis points exceeds maximum (10000).
    FeeTooHigh = 5,
    /// Amount is not positive.
    InvalidAmount = 6,
    /// Amount is below minimum refund amount.
    AmountTooLow = 7,
    /// Arithmetic overflow would occur.
    Overflow = 8,
    /// Refund request is not in the expected status.
    InvalidStatus = 9,
}
