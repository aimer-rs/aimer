#![no_main]

use aimer_cupid_fuzz::fuzz_font_outlines;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| fuzz_font_outlines(data));
