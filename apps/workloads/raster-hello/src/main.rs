extern crate alloc;

use alloc::string::String;
use raster::prelude::*;

#[tile(kind = iter)]
fn multiply(value: u64, factor: u64) -> u64 {
    value.saturating_mul(factor)
}

#[tile(kind = iter)]
fn offset(value: u64, delta: u64) -> u64 {
    value.saturating_add(delta)
}

#[tile(kind = iter)]
fn summarize(label: String, value: u64) -> String {
    format!("{}:{}", label, value)
}

#[tile(kind = iter)]
fn seed_from_name(name: String) -> u64 {
    name.len() as u64
}

#[sequence]
fn compute(seed: u64) -> u64 {
    let stage_1 = call!(multiply, seed, 3);
    let stage_2 = call!(offset, stage_1, 7);
    call!(multiply, stage_2, 2)
}

#[sequence]
fn report(label: String, seed: u64) -> String {
    let total = call_seq!(compute, seed);
    call!(summarize, label, total)
}

#[sequence]
fn main(name: String) {
    let seed = call!(seed_from_name, name);
    let _first = call_seq!(report, "primary".to_string(), seed);
    let next_seed = call!(offset, seed, 1);
    let _second = call_seq!(report, "secondary".to_string(), next_seed);
}
