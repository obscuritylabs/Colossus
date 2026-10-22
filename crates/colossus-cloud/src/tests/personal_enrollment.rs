//! Hosted bootstrap recovery preserves its exact placement and pending certificate.
use super::*;

fn invitation(project: &str, hash: char, node: &str) -> Enrollment {
    Enrollment {
        owner_user_id: None,
        token_hash: hash.to_string().repeat(64),
        project_id: project.into(),
        node_id: node.into(),
        label: "Hosted chat".into(),
        roles: ["primary".into()].into(),
        expires_at: 200,
        redeemed_certificate: None,
        redeemed_csr: None,
        certificate_pem: None,
    }
}

#[tokio::test]
async fn personal_bootstrap_renewal_preserves_identity_and_reconciles_consumed_certificates() {
    let (repo, admin, _) = fixture().await;
    let original = invitation(admin.project_id(), 'b', "chat-alice");
    repo.invite_personal(&admin, original.clone(), "alice", 100)
        .await
        .unwrap();
    let renewed = repo
        .renew_personal_enrollment(
            &admin,
            &original.token_hash,
            "alice",
            "chat-alice",
            400,
            201,
        )
        .await
        .unwrap();
    let mut expected = original.clone();
    expected.owner_user_id = Some("alice".into());
    expected.expires_at = 400;
    assert_eq!(renewed, expected);
    assert_eq!(
        repo.renew_personal_enrollment(
            &admin,
            &original.token_hash,
            "alice",
            "chat-alice",
            400,
            201
        )
        .await
        .unwrap(),
        renewed
    );
    let certificate = || CertificateRedemption {
        fingerprint: "c".repeat(64),
        csr_sha256: "d".repeat(64),
        certificate_pem: "original public certificate".into(),
    };
    let enrolled = repo
        .redeem(&original.token_hash, "hosted-alice", certificate(), 300)
        .await
        .unwrap();
    let consumed = repo
        .renew_personal_enrollment(
            &admin,
            &original.token_hash,
            "alice",
            "chat-alice",
            1200,
            1000,
        )
        .await
        .unwrap();
    assert_eq!(consumed.expires_at, 400);
    assert_eq!(
        consumed.redeemed_certificate.as_deref(),
        Some("c".repeat(64).as_str())
    );
    assert_eq!(
        consumed.redeemed_csr.as_deref(),
        Some("d".repeat(64).as_str())
    );
    assert_eq!(
        consumed.certificate_pem.as_deref(),
        Some("original public certificate")
    );
    assert_eq!(
        repo.redeem(&original.token_hash, "hosted-alice", certificate(), 1001)
            .await
            .unwrap(),
        enrolled
    );
    let mut unrelated = certificate();
    unrelated.csr_sha256 = "e".repeat(64);
    assert_eq!(
        repo.redeem(&original.token_hash, "hosted-alice", unrelated, 1002)
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
}

#[tokio::test]
async fn personal_bootstrap_renewal_rejects_foreign_shared_and_unbounded_invites() {
    let (repo, admin, _) = fixture().await;
    let original = invitation(admin.project_id(), 'b', "chat-alice");
    repo.invite_personal(&admin, original.clone(), "alice", 100)
        .await
        .unwrap();
    let shared = invitation(admin.project_id(), 'c', "shared-node");
    repo.invite(&admin, shared.clone(), 100).await.unwrap();
    let limited = caller(admin.project_id(), &[CloudPermission::Execute]);
    let foreign = caller("foreign-project", &[CloudPermission::Administer]);
    for (caller, hash, owner, node, expiry, expected) in [
        (
            &limited,
            &original.token_hash,
            "alice",
            "chat-alice",
            400,
            CloudError::PermissionDenied,
        ),
        (
            &foreign,
            &original.token_hash,
            "alice",
            "chat-alice",
            400,
            CloudError::NotFound,
        ),
        (
            &admin,
            &shared.token_hash,
            "alice",
            "shared-node",
            400,
            CloudError::NotFound,
        ),
        (
            &admin,
            &original.token_hash,
            "bob",
            "chat-alice",
            400,
            CloudError::NotFound,
        ),
        (
            &admin,
            &original.token_hash,
            "alice",
            "other-node",
            400,
            CloudError::NotFound,
        ),
        (
            &admin,
            &original.token_hash,
            "alice",
            "chat-alice",
            201,
            CloudError::InvalidArgument,
        ),
        (
            &admin,
            &original.token_hash,
            "alice",
            "chat-alice",
            802,
            CloudError::InvalidArgument,
        ),
    ] {
        assert_eq!(
            repo.renew_personal_enrollment(caller, hash, owner, node, expiry, 201)
                .await
                .unwrap_err(),
            expected
        );
    }
}
