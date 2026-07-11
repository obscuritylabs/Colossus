#![no_main]

use colossus_fuzzing::exercise_workflow_yaml;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| exercise_workflow_yaml(data));
