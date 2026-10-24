#![no_main]

use aimer_reload_protocol_fuzz::fuzz_unauthenticated_connection;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| fuzz_unauthenticated_connection(data));