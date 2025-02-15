#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(any(feature = "std", feature = "spin-lock"))]
mod basic;

mod default_impl;
mod errors;
mod generic;
mod matching_eq;

#[cfg(any(feature = "std", feature = "spin-lock"))]
mod matching_pat;

#[cfg(any(feature = "std", feature = "spin-lock"))]
mod mixed;

#[cfg(any(feature = "std", feature = "spin-lock"))]
mod mock_order;

mod prefix;

#[cfg(all(feature = "pretty-print", any(feature = "std", feature = "spin-lock")))]
mod pretty_mismatches;

#[cfg(all(feature = "mock-core", feature = "mock-std"))]
mod std;

mod unmock;

fn main() {}
