// Copyright (c) Zefchain Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

#[cfg(target_arch = "wasm32")]
use std::sync::{Mutex, MutexGuard, OnceLock};

use rand::{rngs::SmallRng, Rng, SeedableRng};

// The following seed is chosen to have equal numbers of 1s and 0s, as advised by
// https://docs.rs/rand/latest/rand/rngs/struct.SmallRng.html
// Specifically, it's "01" × 32 in binary
const RNG_SEED: u64 = 6148914691236517205;

/// A deterministic RNG.
pub type DeterministicRng = SmallRng;

/// A RNG that is non-deterministic if the platform supports it.
#[cfg(not(target_arch = "wasm32"))]
pub type NonDeterministicRng = rand::rngs::ThreadRng;
/// A RNG that is non-deterministic if the platform supports it.
#[cfg(target_arch = "wasm32")]
pub type NonDeterministicRng = MutexGuard<'static, DeterministicRng>;

/// Returns a deterministic RNG for testing.
pub fn make_deterministic_rng() -> DeterministicRng {
    SmallRng::seed_from_u64(RNG_SEED)
}

/// Returns a non-deterministic RNG where supported.
pub fn make_nondeterministic_rng() -> NonDeterministicRng {
    #[cfg(target_arch = "wasm32")]
    {
        static RNG: OnceLock<Mutex<SmallRng>> = OnceLock::new();
        RNG.get_or_init(|| Mutex::new(make_deterministic_rng()))
            .lock()
            .expect("failed to lock RNG mutex")
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        rand::thread_rng()
    }
}

/// Get a random alphanumeric string that can be used for all tests.
pub fn generate_random_alphanumeric_string(length: usize, charset: &[u8]) -> String {
    (0..length)
        .map(|_| {
            let random_index = make_nondeterministic_rng().gen_range(0..charset.len());
            charset[random_index] as char
        })
        .collect()
}

/// Returns a unique namespace for testing.
pub fn generate_test_namespace() -> String {
    // Define the characters that are allowed in the alphanumeric string
    let charset: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let entry = generate_random_alphanumeric_string(20, charset);
    let namespace = format!("table_{}", entry);
    tracing::warn!("Generating namespace={}", namespace);
    namespace
}
