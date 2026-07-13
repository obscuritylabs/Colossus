#![no_main]

use colossus_fuzzing::exercise_contracts_json;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| exercise_contracts_json(data));
