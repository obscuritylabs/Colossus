use super::*;

#[test]
fn corrupt_missing_and_oversized_models_fail_before_loading() {
    let digest = format!("{:x}", Sha256::digest(b"model"));
    assert_eq!(verify_reader(&b"model"[..], &digest, 5).unwrap(), b"model");
    assert_eq!(
        verify_reader(&b"other"[..], &digest, 5),
        Err(DictationError::ModelIntegrity)
    );
    assert_eq!(
        verify_reader(&b""[..], &digest, 5),
        Err(DictationError::ModelUnavailable)
    );
    assert_eq!(
        verify_reader(&b"model!"[..], &digest, 5),
        Err(DictationError::ModelUnavailable)
    );
    assert_eq!(
        verify_reader(&b"model"[..], "not-a-digest", 5),
        Err(DictationError::ModelIntegrity)
    );
    assert_eq!(
        verify(Path::new("absent-dictation-model.bin"), &digest),
        Err(DictationError::ModelUnavailable)
    );
}
