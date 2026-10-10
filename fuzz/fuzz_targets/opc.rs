#![no_main]
//! OPC package: read -> integrity check -> write -> read.

use libfuzzer_sys::fuzz_target;
use open365_opc::{Limits, Package};

fuzz_target!(|data: &[u8]| {
    // Tight limits keep each execution fast; the code path is the same as with defaults.
    let limits = Limits {
        max_parts: 2_000,
        max_part_bytes: 8 * 1024 * 1024,
        max_total_bytes: 32 * 1024 * 1024,
        ..Limits::default()
    };
    if let Ok(package) = Package::read(data, &limits) {
        let _ = package.check_integrity();
        if let Ok(bytes) = package.write() {
            let _ = Package::read(&bytes, &limits);
        }
    }
});
