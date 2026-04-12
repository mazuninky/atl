//! Shared test-only helpers for the `atl` library crate.
//!
//! This module exists so every test that mutates the process-wide
//! environment can serialize through a single mutex. Rust 2024 marks
//! `std::env::set_var` / `remove_var` as `unsafe` because POSIX env
//! access is not thread-safe: any concurrent read or write from another
//! thread (including libc functions like `getaddrinfo` and Rust stdlib
//! helpers like `std::net::ToSocketAddrs`) can race the mutation and
//! produce undefined behaviour.
//!
//! A per-module `OnceLock<Mutex<()>>` only serializes within that
//! module. `cargo test` compiles every `#[cfg(test)]` module into the
//! same lib-test binary, so tests in `src/client`, `src/config`, and
//! `src/io` all run in the same process and must coordinate through
//! the same mutex. Calling [`env_lock`] from every test that touches
//! env vars is enough to satisfy POSIX's "all env access must be
//! synchronized" invariant for same-crate tests.
//!
//! Integration tests (files under `tests/`) compile to separate
//! binaries and therefore have their own process; they can keep their
//! own local lock if they need one.

/// Acquire the single process-wide mutex guarding test env-var
/// mutations. Holding the returned guard while calling
/// `unsafe { std::env::set_var(...) }` / `remove_var(...)` — and while
/// reading any env var whose value a concurrent test could have just
/// written — keeps same-crate lib tests from racing each other.
///
/// Uses `unwrap_or_else(|e| e.into_inner())` so a poisoned mutex (from
/// a prior panicking test) does not cascade into further failures; the
/// mutex protects a unit `()` so there is no invariant to preserve.
pub(crate) fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}
