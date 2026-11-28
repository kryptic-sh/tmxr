#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| tmxr_fuzz::key_names(data));
