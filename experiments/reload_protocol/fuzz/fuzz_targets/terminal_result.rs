#![no_main]

use aimer_reload_protocol_fuzz::fuzz_terminal_result;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| fuzz_terminal_result(data));