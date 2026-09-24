//! Native, user-managed mail between independent resumable root threads.

mod extension;
mod store;
mod tools;

pub use extension::install;
pub use store::GroupMailStore;
pub use store::MemberStatus;
pub use store::Membership;
pub use store::Priority;

#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;
