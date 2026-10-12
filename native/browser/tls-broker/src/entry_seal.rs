//! Strict build-time code identity. This is not a runtime authority selector.

pub fn parse(input: &str) -> Result<[u8; 20], ()> {
    if input.len() != 40 || !input.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(());
    }
    let mut hash = [0_u8; 20];
    for (output, pair) in hash.iter_mut().zip(input.as_bytes().chunks_exact(2)) {
        let pair = std::str::from_utf8(pair).map_err(|_| ())?;
        *output = u8::from_str_radix(pair, 16).map_err(|_| ())?;
    }
    if hash == [0; 20] {
        return Err(());
    }
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_nonzero_code_hashes_are_accepted() {
        assert_eq!(
            parse("0123456789abcdef0123456789abcdef01234567").unwrap(),
            [
                1, 35, 69, 103, 137, 171, 205, 239, 1, 35, 69, 103, 137, 171, 205, 239, 1, 35, 69,
                103
            ]
        );
        assert_eq!(
            parse("ABCDEFABCDEFABCDEFABCDEFABCDEFABCDEFABCD").unwrap()[0],
            0xab
        );
        for invalid in [
            "",
            "1",
            "0000000000000000000000000000000000000000",
            "g123456789abcdef0123456789abcdef01234567",
            "0123456789abcdef0123456789abcdef01234567\n",
            " 0123456789abcdef0123456789abcdef01234567",
            "cdhash=0123456789abcdef0123456789abcdef01234567",
        ] {
            assert!(parse(invalid).is_err());
        }
    }
}
