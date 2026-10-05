#![no_std]
//! Callora emergency capability surface.
//!
//! This crate exposes a **read-only** [`views::capabilities`] bitmap so
//! clients can detect which emergency features a deployment supports without
//! parsing version strings.  The bitmap is stable across upgrades: bits are
//! assigned once and never reassigned.
//!
//! # Quick-start
//! ```ignore
//! let caps = client.capabilities();
//! if caps & CAP_EMERGENCY_PAUSE != 0 {
//!     // safe to call pause() / unpause()
//! }
//! ```
//!
//! # Capability-delta detection
//! ```ignore
//! let before = old_client.capabilities();
//! let after  = new_client.capabilities();
//! let added   = after & !before;
//! let removed = before & !after;
//! ```

pub mod migrate;
mod views;

pub use views::{
    capabilities, ALL_CAPABILITIES, CAP_EMERGENCY_DRAIN_CANCEL, CAP_EMERGENCY_DRAIN_EXECUTE,
    CAP_EMERGENCY_DRAIN_PROPOSE, CAP_EMERGENCY_PAUSE, CAP_EMERGENCY_UNPAUSE,
    CAP_PENDING_DRAIN_VIEW,
};

use soroban_sdk::{contract, contractimpl, Env};

/// Thin contract facade that exposes emergency capability discovery on-chain.
///
/// The emergency operations themselves (pause, drain, etc.) live in the
/// revenue-pool contract; this entrypoint exists so integrators and
/// capability-delta monitors have a stable `capabilities()` view keyed to
/// the emergency feature set.
#[contract]
pub struct CalloraEmergency;

#[contractimpl]
impl CalloraEmergency {
    /// Return the emergency-feature capability bitmap for this deployment.
    ///
    /// Each set bit signals a supported emergency feature.  Bits are stable
    /// across upgrades — once assigned a bit position is never reused.
    /// Reserved bits (6–63) are always zero.
    ///
    /// Pure view: no auth, no storage writes, no TTL bump.
    pub fn capabilities(env: Env) -> u64 {
        views::capabilities(&env)
    }
}
