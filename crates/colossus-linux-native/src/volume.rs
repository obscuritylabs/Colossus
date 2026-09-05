use crate::identity::{NfsVolumeIdentity, ServerAddress};
use std::io;

pub(crate) const MAX_NFS_VOLUMES_BYTES: usize = 1024 * 1024;
const MAX_NFS_VOLUME_LINE_BYTES: usize = 256;
const MAX_NFS_VOLUME_RECORDS: usize = 4096;
const EXPECTED_HEADER: [&str; 6] = ["NV", "SERVER", "PORT", "DEV", "FSID", "FSC"];

pub(crate) fn parse_matching_volume(
    contents: &[u8],
    device_major: u32,
    device_minor: u32,
) -> io::Result<NfsVolumeIdentity> {
    if contents.len() > MAX_NFS_VOLUMES_BYTES || !contents.is_ascii() {
        return Err(malformed_table(None));
    }
    let text = std::str::from_utf8(contents).map_err(|_| malformed_table(None))?;
    let mut lines = text.split_terminator('\n');
    let header = lines.next().ok_or_else(|| malformed_table(None))?;
    if header.len() > MAX_NFS_VOLUME_LINE_BYTES
        || !header.split_ascii_whitespace().eq(EXPECTED_HEADER)
    {
        return Err(malformed_table(Some(1)));
    }

    let mut matching = None;
    let mut matching_count = 0_usize;
    for (record_index, line) in lines.enumerate() {
        let line_number = record_index + 2;
        if record_index >= MAX_NFS_VOLUME_RECORDS
            || line.is_empty()
            || line.len() > MAX_NFS_VOLUME_LINE_BYTES
        {
            return Err(malformed_table(Some(line_number)));
        }
        let record = parse_record(line, line_number)?;
        if record.device_major == device_major && record.device_minor == device_minor {
            matching_count += 1;
            matching = Some(record);
        }
    }

    match (matching_count, matching) {
        (1, Some(record)) => parse_identity(record),
        (0, _) => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Linux NFS volume table has no exact device match",
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Linux NFS volume table has multiple exact device matches",
        )),
    }
}

struct VolumeRecord<'a> {
    line_number: usize,
    version: &'a str,
    server: &'a str,
    port: &'a str,
    fsid: &'a str,
    fscache: &'a str,
    device_major: u32,
    device_minor: u32,
}

fn parse_record(line: &str, line_number: usize) -> io::Result<VolumeRecord<'_>> {
    let mut fields = line.split_ascii_whitespace();
    let version = fields
        .next()
        .ok_or_else(|| malformed_table(Some(line_number)))?;
    let server = fields
        .next()
        .ok_or_else(|| malformed_table(Some(line_number)))?;
    let port = fields
        .next()
        .ok_or_else(|| malformed_table(Some(line_number)))?;
    let device = fields
        .next()
        .ok_or_else(|| malformed_table(Some(line_number)))?;
    let fsid = fields
        .next()
        .ok_or_else(|| malformed_table(Some(line_number)))?;
    let fscache = fields
        .next()
        .ok_or_else(|| malformed_table(Some(line_number)))?;
    if fields.next().is_some() {
        return Err(malformed_table(Some(line_number)));
    }
    let (device_major, device_minor) =
        parse_device(device).ok_or_else(|| malformed_table(Some(line_number)))?;

    Ok(VolumeRecord {
        line_number,
        version,
        server,
        port,
        fsid,
        fscache,
        device_major,
        device_minor,
    })
}

fn parse_identity(record: VolumeRecord<'_>) -> io::Result<NfsVolumeIdentity> {
    let line_number = record.line_number;
    if !matches!(record.fscache, "yes" | "no") {
        return Err(malformed_table(Some(line_number)));
    }
    let nfs_version = match record.version {
        "v2" => 2,
        "v3" => 3,
        "v4" => 4,
        _ => return Err(malformed_table(Some(line_number))),
    };
    let server_address =
        parse_server_address(record.server).ok_or_else(|| malformed_table(Some(line_number)))?;
    let server_port = parse_canonical_hex(record.port, 4)
        .and_then(|value| u16::try_from(value).ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| malformed_table(Some(line_number)))?;
    let (fsid_major, fsid_minor) =
        parse_fsid(record.fsid).ok_or_else(|| malformed_table(Some(line_number)))?;

    Ok(NfsVolumeIdentity {
        nfs_version,
        server_address,
        server_port,
        fsid_major,
        fsid_minor,
    })
}

fn parse_server_address(value: &str) -> Option<ServerAddress> {
    match value.len() {
        8 => decode_hex::<4>(value)
            .filter(|address| *address != [0; 4])
            .map(ServerAddress::Ipv4),
        32 => decode_hex::<16>(value)
            .filter(|address| *address != [0; 16] && !is_ipv6_link_local(address))
            .map(ServerAddress::Ipv6),
        _ => None,
    }
}

const fn is_ipv6_link_local(address: &[u8; 16]) -> bool {
    address[0] == 0xfe && address[1] & 0xc0 == 0x80
}

fn decode_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut decoded = [0_u8; N];
    for (destination, pair) in decoded.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        *destination = hex_nibble(pair[0])?
            .checked_mul(16)?
            .checked_add(hex_nibble(pair[1])?)?;
    }
    Some(decoded)
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn parse_device(value: &str) -> Option<(u32, u32)> {
    let (major, minor) = split_pair(value)?;
    Some((
        parse_canonical_decimal(major)?,
        parse_canonical_decimal(minor)?,
    ))
}

fn parse_fsid(value: &str) -> Option<(u64, u64)> {
    let (major, minor) = split_pair(value)?;
    Some((
        parse_canonical_hex(major, 16)?,
        parse_canonical_hex(minor, 16)?,
    ))
}

fn split_pair(value: &str) -> Option<(&str, &str)> {
    let (left, right) = value.split_once(':')?;
    if right.contains(':') {
        return None;
    }
    Some((left, right))
}

fn parse_canonical_decimal(value: &str) -> Option<u32> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    value.parse().ok()
}

fn parse_canonical_hex(value: &str, max_digits: usize) -> Option<u64> {
    if value.is_empty()
        || value.len() > max_digits
        || (value.len() > 1 && value.starts_with('0'))
        || !value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    u64::from_str_radix(value, 16).ok()
}

fn malformed_table(line_number: Option<usize>) -> io::Error {
    let message = line_number.map_or_else(
        || "Linux NFS volume table is malformed".to_owned(),
        |line| format!("Linux NFS volume table is malformed at line {line}"),
    );
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::{MAX_NFS_VOLUME_RECORDS, MAX_NFS_VOLUMES_BYTES, parse_matching_volume};

    const HEADER: &str = "NV SERVER PORT DEV FSID FSC\n";

    #[test]
    fn parses_ipv4_volume_into_canonical_binary_fields() {
        let input = format!("{HEADER}v4 7f000001 801 0:51 1a:2b no\n");

        let identity = parse_matching_volume(input.as_bytes(), 0, 51).unwrap();

        assert_eq!(identity.nfs_version, 4);
        assert_eq!(identity.server_address.as_bytes(), [127, 0, 0, 1]);
        assert_eq!(identity.server_port, 2049);
        assert_eq!(identity.fsid_major, 0x1a);
        assert_eq!(identity.fsid_minor, 0x2b);
    }

    #[test]
    fn parses_ipv6_volume_into_canonical_binary_fields() {
        let input = format!(
            "{HEADER}v3 20010db8000000000000000000000001 7ff 12:34 123456789abcdef0:0 yes\n"
        );

        let identity = parse_matching_volume(input.as_bytes(), 12, 34).unwrap();

        assert_eq!(
            identity.server_address.as_bytes(),
            [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,]
        );
        assert_eq!(identity.server_port, 2047);
        assert_eq!(identity.fsid_major, 0x1234_5678_9abc_def0);
        assert_eq!(identity.fsid_minor, 0);
    }

    #[test]
    fn selects_only_the_exact_device_match() {
        let input = format!("{HEADER}v4 c0000201 801 1:2 a:b no\nv4 c0000202 801 1:20 c:d no\n");

        let identity = parse_matching_volume(input.as_bytes(), 1, 2).unwrap();

        assert_eq!(identity.server_address.as_bytes(), [192, 0, 2, 1]);
    }

    #[test]
    fn ignores_unrelated_link_local_mounts_before_or_after_the_match() {
        let matching = "v4 c0000201 801 0:51 1:2 no\n";
        let unrelated = "v4 fe800000000000000000000000000001 801 0:52 3:4 no\n";
        let expected =
            parse_matching_volume(format!("{HEADER}{matching}").as_bytes(), 0, 51).unwrap();

        for records in [
            format!("{unrelated}{matching}"),
            format!("{matching}{unrelated}"),
        ] {
            let input = format!("{HEADER}{records}");
            assert_eq!(
                parse_matching_volume(input.as_bytes(), 0, 51).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn rejects_matching_link_local_mount() {
        let input = format!("{HEADER}v4 fe800000000000000000000000000001 801 0:51 1:2 no\n");

        assert_eq!(
            parse_matching_volume(input.as_bytes(), 0, 51)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidData,
        );
    }

    #[test]
    fn rejects_malformed_device_or_field_shape_on_unrelated_records() {
        let invalid_records = [
            "v4 c0000202 801 00:52 3:4 no",
            "v4 c0000202 801 0:52:1 3:4 no",
            "v4 c0000202 801 not-a-device 3:4 no",
            "v4 c0000202 801 0:52 3:4",
            "v4 c0000202 801 0:52 3:4 no extra",
        ];

        for record in invalid_records {
            let input = format!("{HEADER}v4 c0000201 801 0:51 1:2 no\n{record}\n");
            assert_eq!(
                parse_matching_volume(input.as_bytes(), 0, 51)
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::InvalidData,
            );
        }
    }

    #[test]
    fn rejects_duplicate_matching_devices_even_with_unsupported_scope() {
        let matching = "v4 c0000201 801 0:51 1:2 no\n";
        let unsupported = "v4 fe800000000000000000000000000001 801 0:51 3:4 no\n";

        for records in [
            format!("{matching}{matching}"),
            format!("{matching}{unsupported}"),
            format!("{unsupported}{matching}"),
        ] {
            let input = format!("{HEADER}{records}");
            let error = parse_matching_volume(input.as_bytes(), 0, 51).unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("multiple exact device matches"));
        }
    }

    #[test]
    fn rejects_absent_and_ambiguous_device_matches() {
        let one_record = format!("{HEADER}v4 c0000201 801 1:2 a:b no\n");
        let missing = parse_matching_volume(one_record.as_bytes(), 9, 9).unwrap_err();
        assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);

        let duplicate =
            format!("{HEADER}v4 c0000201 801 1:2 a:b no\nv4 c0000202 801 1:2 c:d yes\n");
        let ambiguous = parse_matching_volume(duplicate.as_bytes(), 1, 2).unwrap_err();
        assert_eq!(ambiguous.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn rejects_noncanonical_or_unknown_fields() {
        let invalid_records = [
            "v5 c0000201 801 1:2 a:b no",
            "v4 C0000201 801 1:2 a:b no",
            "v4 00000000 801 1:2 a:b no",
            "v4 00000000000000000000000000000000 801 1:2 a:b no",
            "v4 fe800000000000000000000000000001 801 1:2 a:b no",
            "v4 c0000201 0 1:2 a:b no",
            "v4 c0000201 0801 1:2 a:b no",
            "v4 c0000201 801 01:2 a:b no",
            "v4 c0000201 801 1:2 0a:b no",
            "v4 c0000201 801 1:2 a:b maybe",
            "v4 c0000201 801 1:2 a:b no extra",
        ];

        for record in invalid_records {
            let input = format!("{HEADER}{record}\n");
            assert!(parse_matching_volume(input.as_bytes(), 1, 2).is_err());
        }
    }

    #[test]
    fn rejects_wrong_header_and_non_ascii_data() {
        let wrong_header = b"NV SERVER PORT DEV FSID\nv4 c0000201 801 1:2 a:b no\n";
        assert!(parse_matching_volume(wrong_header, 1, 2).is_err());

        let mut non_ascii = format!("{HEADER}v4 c0000201 801 1:2 a:b no\n").into_bytes();
        non_ascii.push(0xff);
        assert!(parse_matching_volume(&non_ascii, 1, 2).is_err());
    }

    #[test]
    fn bounds_table_bytes_and_record_count() {
        let oversized = vec![b'x'; MAX_NFS_VOLUMES_BYTES + 1];
        assert!(parse_matching_volume(&oversized, 1, 2).is_err());

        let mut too_many = String::from(HEADER);
        for minor in 0..=MAX_NFS_VOLUME_RECORDS {
            too_many.push_str(&format!("v4 c0000201 801 1:{minor} a:b no\n"));
        }
        assert!(too_many.len() < MAX_NFS_VOLUMES_BYTES);
        assert!(parse_matching_volume(too_many.as_bytes(), 99, 99).is_err());
    }
}
