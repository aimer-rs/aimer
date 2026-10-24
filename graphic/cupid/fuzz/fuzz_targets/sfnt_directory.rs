#![no_main]

use aimer_cupid_fuzz::fuzz_sfnt_directory;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| fuzz_sfnt_directory(data));
