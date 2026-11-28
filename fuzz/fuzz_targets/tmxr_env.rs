#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| tmxr_fuzz::tmxr_env(data));
