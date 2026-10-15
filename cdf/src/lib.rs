//! CDF v1 GPT profile (design/README.md).
//!
//! Shared by the `cdf-gpt` host tool and the guest opener (CDH): on-disk
//! format, key hierarchy, GPT layout, and the create / open state machine.
//!
//! ```text
//!   Blank --create--> Sealed --open--> Mapped
//! ```
//!
//! `create` writes the authenticated metadata last, so a volume is either
//! blank (no valid Superblock) or fully sealed; there is no authenticated
//! "sealed but not yet populated" state.

pub mod device;
pub mod format;
pub mod fs;
pub mod gpt_layout;
pub mod keys;
pub mod luks;
pub mod volume;

pub use format::Profile;
pub use keys::RootKey;
pub use volume::{
    activate, create, inspect, probe, verify, Activated, CreateRequest, Created, DiskState,
    Inspected, Verified,
};
