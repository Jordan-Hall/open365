#![no_main]
//! OPC zip open: zip bombs, odd part names, duplicate entries, lying sizes. Small limits keep
//! the fuzzer fast; a panic, hang or out-of-memory is a finding.

use libfuzzer_sys::fuzz_target;
use open365_opc::{Limits, Package};

fuzz_target!(|data: &[u8]| {
    let limits = Limits {
        max_parts: 2_000,
        max_part_bytes: 8 * 1024 * 1024,
        max_total_bytes: 32 * 1024 * 1024,
        max_ratio: 200,
        ratio_floor_bytes: 1024 * 1024,
    };
    if let Ok(package) = Package::read(data, &limits) {
        let _ = package.write();
    }
});
