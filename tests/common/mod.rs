// Test files under `tests/` each compile this module as a separate copy,
// so a re-export that one file uses but another doesn't will trip
// `unused_imports`. Both lints are silenced at the module level so adding
// new test files doesn't require touching this shared helper.
#![allow(dead_code, unused_imports)]

pub mod atl;
pub mod config;
pub mod prism;

pub use atl::AtlRunner;
pub use config::{TestConfig, TestConfigBuilder};
pub use prism::PrismServer;
