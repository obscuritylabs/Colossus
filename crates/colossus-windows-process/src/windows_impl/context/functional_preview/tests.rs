use super::*;

fn ordinary() -> Token {
    Token {
        user: Vec::new(),
        session: 1,
        authentication: [1, 0],
        elevated: false,
        appcontainer: false,
        integrity: 8192,
        capabilities: 0,
        package: None,
    }
}

#[test]
fn functional_preview_denies_elevation_isolation_and_noninteractive_tokens() {
    assert!(validate_token(&ordinary()).is_ok());
    for changed in [
        Token {
            elevated: true,
            ..ordinary()
        },
        Token {
            appcontainer: true,
            ..ordinary()
        },
        Token {
            integrity: 4096,
            ..ordinary()
        },
        Token {
            integrity: 8448,
            ..ordinary()
        },
        Token {
            integrity: 12288,
            ..ordinary()
        },
        Token {
            session: 0,
            ..ordinary()
        },
        Token {
            capabilities: 1,
            ..ordinary()
        },
        Token {
            package: Some(vec![1]),
            ..ordinary()
        },
    ] {
        assert!(validate_token(&changed).is_err());
    }
}
