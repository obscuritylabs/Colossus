//! Literal private protocol oracles; these tests make no native process effects.
use super::*;
use std::{ffi::OsString, os::windows::ffi::OsStringExt as _};

fn arguments(values: &[&str]) -> Vec<OsString> {
    values.iter().map(|value| OsString::from(*value)).collect()
}

#[test]
fn ordinary_browser_and_cef_arguments_leave_helper_dispatch_absent() {
    for values in [
        vec![],
        vec!["--type=renderer"],
        vec!["--type=gpu-process", "--lang=en-US"],
        vec![
            "--inherited-browser-pipes",
            "4",
            "8",
            "12",
            "16",
            "20",
            "24",
        ],
    ] {
        assert!(parse_values(arguments(&values)).is_none());
    }
}

#[test]
fn helper_arguments_accept_only_fixed_complete_roles_and_reject_mixed_modes() {
    let literal = [
        "--colossus-browser-context-v1",
        "launcher",
        "0123456789abcdef0123456789abcdef",
        "42",
        "123456789",
        "987",
        "fedcba98765432100123456789abcdef",
    ];
    for (role, expected) in [("launcher", Role::Launcher), ("context", Role::Context)] {
        let mut values = literal;
        values[1] = role;
        let parsed = parse_values(arguments(&values)).unwrap().unwrap();
        assert!(parsed.role == expected);
        assert_eq!(parsed.core_pid, 42);
        assert_eq!(parsed.core_birth, 123_456_789);
        assert_eq!(parsed.core_image.volume_serial_number, 987);
        assert_eq!(
            parsed.nonce,
            [
                0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef
            ]
        );
        assert_eq!(
            parsed.core_image.file_id,
            [
                0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab,
                0xcd, 0xef
            ]
        );
    }
    for (index, replacement) in [
        (0, "--colossus-browser-context-v2"),
        (1, "browser"),
        (2, "0123456789ABCDEF0123456789abcdef"),
        (2, "g123456789abcdef0123456789abcdef"),
        (2, "0123456789abcdef"),
        (3, "0"),
        (3, "4294967296"),
        (4, "0"),
        (5, "not-a-number"),
        (6, "fedcba98765432100123456789abcdeF"),
        (6, "fedcba98765432100123456789abcdeg"),
        (6, "fedcba9876543210"),
    ] {
        let mut values = literal;
        values[index] = replacement;
        assert!(matches!(parse_values(arguments(&values)), Some(Err(_))));
    }
    for extra in ["--type=renderer", "--colossus-browser-context-v1"] {
        let mut values = arguments(&literal);
        values.push(extra.into());
        assert!(matches!(parse_values(values), Some(Err(_))));
    }
    assert!(matches!(
        parse_values(arguments(&literal[..6])),
        Some(Err(_))
    ));
    let mut non_unicode = arguments(&literal);
    non_unicode[2] = OsString::from_wide(&[0xd800]);
    assert!(matches!(parse_values(non_unicode), Some(Err(_))));
}

// Independently specified wire: package S-1-15-2-1; argv [雪😀, --quiet, ""];
// sorted environment A=é, E="", Z=😀. Lengths are UTF-8 bytes, not UTF-16 units.
const UNICODE_WIRE: &[u8] = &[
    10, 0, 0, 0, b'S', b'-', b'1', b'-', b'1', b'5', b'-', b'2', b'-', b'1', 3, 0, 0, 0, 7, 0, 0,
    0, 0xe9, 0x9b, 0xaa, 0xf0, 0x9f, 0x98, 0x80, 7, 0, 0, 0, b'-', b'-', b'q', b'u', b'i', b'e',
    b't', 0, 0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0, b'A', 2, 0, 0, 0, 0xc3, 0xa9, 1, 0, 0, 0, b'E', 0, 0,
    0, 0, 1, 0, 0, 0, b'Z', 4, 0, 0, 0, 0xf0, 0x9f, 0x98, 0x80,
];

fn request() -> SpawnRequest {
    SpawnRequest {
        executable: r"C:\component\colossus-native-browser-host.exe".into(),
        cwd: r"C:\component".into(),
        arguments: vec!["雪😀".into(), "--quiet".into(), String::new()],
        environment: BTreeMap::from([
            ("A".into(), "é".into()),
            ("E".into(), String::new()),
            ("Z".into(), "😀".into()),
        ]),
        appcontainer_sid: "S-1-15-2-1".into(),
        max_processes: 70,
        max_memory_bytes: 2 * 1024 * 1024 * 1024,
        proxy_port: None,
        network_filter_id: None,
    }
}

#[test]
fn browser_envelope_preserves_literal_unicode_arguments_and_environment() {
    let decoded = BrowserValues::decode(UNICODE_WIRE).unwrap();
    assert_eq!(decoded.package, "S-1-15-2-1");
    assert_eq!(decoded.arguments, ["雪😀", "--quiet", ""]);
    assert_eq!(decoded.environment.get("A").unwrap(), "é");
    assert_eq!(decoded.environment.get("E").unwrap(), "");
    assert_eq!(decoded.environment.get("Z").unwrap(), "😀");
    assert_eq!(
        decoded
            .environment
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["A", "E", "Z"]
    );
    assert_eq!(BrowserValues::encode(&request()).unwrap(), UNICODE_WIRE);

    let mut empty_wire = UNICODE_WIRE[..14].to_vec();
    empty_wire.extend_from_slice(&[0; 8]); // independently literal zero argv/env counts
    let empty = BrowserValues::decode(&empty_wire).unwrap();
    assert!(empty.arguments.is_empty() && empty.environment.is_empty());
    let mut empty_request = request();
    empty_request.arguments.clear();
    empty_request.environment.clear();
    assert_eq!(BrowserValues::encode(&empty_request).unwrap(), empty_wire);
}

#[test]
fn browser_envelope_rejects_ambiguous_truncated_and_oversized_values() {
    for end in 0..UNICODE_WIRE.len() {
        assert!(
            BrowserValues::decode(&UNICODE_WIRE[..end]).is_err(),
            "prefix={end}"
        );
    }
    let mut trailing = UNICODE_WIRE.to_vec();
    trailing.push(0);
    assert!(BrowserValues::decode(&trailing).is_err());
    assert!(BrowserValues::decode(&vec![0; MAX_ENVELOPE + 1]).is_err());
    for declared in [131_069_u32, u32::MAX] {
        let mut bytes = UNICODE_WIRE.to_vec();
        bytes[..4].copy_from_slice(&declared.to_le_bytes());
        assert!(BrowserValues::decode(&bytes).is_err());
    }
    let mut environment_count = UNICODE_WIRE.to_vec();
    environment_count[44..48].copy_from_slice(&32_768_u32.to_le_bytes());
    assert!(BrowserValues::decode(&environment_count).is_err());
    let mut argument_count = UNICODE_WIRE[..14].to_vec();
    argument_count.extend_from_slice(&257_u32.to_le_bytes());
    argument_count.extend_from_slice(&[0; 4].repeat(257));
    argument_count.extend_from_slice(&[0; 4]);
    assert!(BrowserValues::decode(&argument_count).is_err());
    let mut many_arguments = request();
    many_arguments.arguments = vec![String::new(); 257];
    assert!(BrowserValues::encode(&many_arguments).is_err());

    assert_eq!(UNICODE_WIRE[52], b'A');
    assert_eq!(UNICODE_WIRE[72], b'Z');
    let mut duplicate = UNICODE_WIRE.to_vec();
    duplicate[72] = b'A';
    assert!(BrowserValues::decode(&duplicate).is_err());
    let mut empty_key = UNICODE_WIRE.to_vec();
    empty_key[48..52].fill(0);
    empty_key.remove(52);
    assert!(BrowserValues::decode(&empty_key).is_err());
    for (index, value) in [(4, b'X'), (22, 0xff), (33, 0), (52, 0), (52, b'=')] {
        let mut bytes = UNICODE_WIRE.to_vec();
        bytes[index] = value;
        assert!(BrowserValues::decode(&bytes).is_err(), "offset={index}");
    }
    let mut nul_value = UNICODE_WIRE.to_vec();
    nul_value[77..81].copy_from_slice(&[0, b'a', b'b', b'c']);
    assert!(BrowserValues::decode(&nul_value).is_err());
    for (key, value) in [
        ("", "ok"),
        ("A=B", "ok"),
        ("A\0B", "ok"),
        ("A", "bad\0value"),
    ] {
        let mut values = request();
        values.environment = BTreeMap::from([(key.into(), value.into())]);
        assert!(BrowserValues::encode(&values).is_err());
    }
    let mut nul_argument = request();
    nul_argument.arguments = vec!["bad\0argument".into()];
    assert!(BrowserValues::encode(&nul_argument).is_err());
    let mut large_string = request();
    large_string.arguments = vec!["a".repeat(131_069)];
    assert!(BrowserValues::encode(&large_string).is_err());
}

#[test]
fn browser_envelope_rejects_helper_and_cef_subprocess_role_arguments() {
    for (argument, literal_record) in [
        ("--type", b"\x06\0\0\0--type".as_slice()),
        ("--type=renderer", b"\x0f\0\0\0--type=renderer".as_slice()),
        (
            "--colossus-browser-context-v1",
            b"\x1d\0\0\0--colossus-browser-context-v1".as_slice(),
        ),
        (
            "--colossus-browser-context-v2",
            b"\x1d\0\0\0--colossus-browser-context-v2".as_slice(),
        ),
        (
            "--inherited-browser-pipes",
            b"\x19\0\0\0--inherited-browser-pipes".as_slice(),
        ),
        (
            "--inherited-browser-pipes=1",
            b"\x1b\0\0\0--inherited-browser-pipes=1".as_slice(),
        ),
    ] {
        assert_eq!(&literal_record[4..], argument.as_bytes());
        assert_eq!(
            u32::from_le_bytes(literal_record[..4].try_into().unwrap()) as usize,
            argument.len()
        );
        let mut values = request();
        values.arguments = vec![argument.into()];
        assert!(BrowserValues::encode(&values).is_err());
        // Literal package prefix + one argument record + zero environment count.
        let mut bytes = UNICODE_WIRE[..14].to_vec();
        bytes.extend_from_slice(&[1, 0, 0, 0]);
        bytes.extend_from_slice(literal_record);
        bytes.extend_from_slice(&[0; 4]);
        assert!(BrowserValues::decode(&bytes).is_err());
    }
}
