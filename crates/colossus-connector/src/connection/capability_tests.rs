use super::connection_capabilities;
use colossus_cloud_protocol::TRANSCRIPT_CAPABILITY;

#[test]
fn transcript_advertisement_requires_current_caller_bound_resources() {
    let stale = vec![TRANSCRIPT_CAPABILITY.into(), TRANSCRIPT_CAPABILITY.into()];
    assert!(
        !connection_capabilities(&stale, &[], false)
            .iter()
            .any(|capability| capability == TRANSCRIPT_CAPABILITY)
    );
    let advertised = connection_capabilities(&stale, &stale, false);
    assert_eq!(
        advertised
            .iter()
            .filter(|capability| *capability == TRANSCRIPT_CAPABILITY)
            .count(),
        1
    );
    assert_eq!(connection_capabilities(&[], &stale, false), advertised);
}
