#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| tmxr_fuzz::tmux_conf(data));
