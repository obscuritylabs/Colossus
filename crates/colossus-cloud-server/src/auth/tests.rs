use super::*;
use crate::config::{Membership, OidcConfig, Storage};
use axum::{
    Form, Json, Router,
    extract::State,
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use colossus_cloud::CloudPermission;
use p256::ecdsa::{Signature, SigningKey, signature::Signer as _};
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
mod transport;

struct IssuerState {
    issuer: String,
    key: SigningKey,
    nonce: Mutex<String>,
    challenge: Mutex<String>,
    invalid: Mutex<&'static str>,
}
async fn discovery(State(state): State<Arc<IssuerState>>) -> Json<Value> {
    Json(
        json!({"issuer":state.issuer,"authorization_endpoint":format!("{}/authorize",state.issuer),"token_endpoint":format!("{}/token",state.issuer),"jwks_uri":format!("{}/jwks",state.issuer),"response_types_supported":["code"],"subject_types_supported":["public"],"id_token_signing_alg_values_supported":["ES256"],"code_challenge_methods_supported":["S256"]}),
    )
}
async fn jwks(State(state): State<Arc<IssuerState>>) -> Json<Value> {
    let point = signing_key(&state).verifying_key().to_encoded_point(false);
    Json(
        json!({"keys":[{"kty":"EC","crv":"P-256","kid":"test","use":"sig","alg":"ES256","x":URL_SAFE_NO_PAD.encode(point.x().unwrap()),"y":URL_SAFE_NO_PAD.encode(point.y().unwrap())}]}),
    )
}
async fn token(
    State(state): State<Arc<IssuerState>>,
    Form(form): Form<HashMap<String, String>>,
) -> Json<Value> {
    assert_eq!(
        URL_SAFE_NO_PAD.encode(Sha256::digest(form["code_verifier"].as_bytes())),
        *state.challenge.lock().unwrap()
    );
    assert_eq!(form["grant_type"], "authorization_code");
    let invalid = *state.invalid.lock().unwrap();
    let now = crate::http::now();
    let mut claims = json!({"iss":state.issuer,"sub":"alice","aud":"colossus","exp":now+300,"iat":now,"nonce":state.nonce.lock().unwrap().clone(),"at_hash":URL_SAFE_NO_PAD.encode(&Sha256::digest(b"access")[..16])});
    match invalid {
        "nonce" => claims["nonce"] = json!("wrong"),
        "audience" => claims["aud"] = json!("other-client"),
        "issuer" => claims["iss"] = json!("https://other.invalid"),
        "expiry" => claims["exp"] = json!(now - 60),
        "subject" => claims["sub"] = json!("not-a-member"),
        "access-hash" => claims["at_hash"] = json!("incorrect"),
        _ => {}
    }
    let header =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"alg":"ES256","kid":"test"})).unwrap());
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
    let message = format!("{header}.{payload}");
    let signature: Signature = signing_key(&state).sign(message.as_bytes());
    let signature = if invalid == "signature" {
        vec![0u8; 64]
    } else {
        signature.to_bytes().to_vec()
    };
    Json(
        json!({"access_token":"access","token_type":"Bearer","expires_in":300,"id_token":format!("{message}.{}",URL_SAFE_NO_PAD.encode(signature))}),
    )
}
fn signing_key(state: &IssuerState) -> SigningKey {
    if *state.invalid.lock().unwrap() == "rotated-key" {
        SigningKey::from_bytes((&[8u8; 32]).into()).unwrap()
    } else {
        state.key.clone()
    }
}

#[tokio::test]
async fn oidc_key_rotation_is_observed_without_host_restart() {
    let (auth, issuer, server) = fixture().await;
    *issuer.invalid.lock().unwrap() = "rotated-key";
    let (headers, state) = flow(&auth, &issuer);
    let cookie = auth
        .callback(&headers, &state, "code".into())
        .await
        .unwrap();
    assert!(cookie.to_str().unwrap().starts_with("colossus_session="));
    server.abort();
}
async fn fixture() -> (
    Authentication,
    Arc<IssuerState>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let state = Arc::new(IssuerState {
        issuer: issuer.clone(),
        key: SigningKey::from_bytes((&[7u8; 32]).into()).unwrap(),
        nonce: Mutex::new(String::new()),
        challenge: Mutex::new(String::new()),
        invalid: Mutex::new(""),
    });
    let router = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/token", post(token))
        .with_state(state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let auth = Authentication::new(Config {
        http_bind: "127.0.0.1:0".parse().unwrap(),
        grpc_bind: "127.0.0.1:0".parse().unwrap(),
        public_origin: "http://127.0.0.1:5180".into(),
        grpc_endpoint: "https://localhost:8443".into(),
        ca_certificate: "ca.pem".into(),
        ca_key: "ca-key.pem".into(),
        server_certificate: "server.pem".into(),
        server_key: "server-key.pem".into(),
        web_root: "dist".into(),
        oidc: OidcConfig {
            issuer,
            client_id: "colossus".into(),
            client_secret_file: None,
        },
        memberships: vec![Membership {
            subject: "alice".into(),
            project_id: "project-a".into(),
            permissions: BTreeSet::from([CloudPermission::Read, CloudPermission::Execute]),
        }],
        storage: Storage::Redb {
            path: "cloud.redb".into(),
            key_variable: None,
        },
        local_development: true,
        signing_key_variable: None,
    })
    .await
    .unwrap();
    (auth, state, task)
}
fn flow(auth: &Authentication, issuer: &IssuerState) -> (HeaderMap, String) {
    let (url, cookie) = auth.login().unwrap();
    let url = url::Url::parse(&url).unwrap();
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["code_challenge_method"], "S256");
    *issuer.challenge.lock().unwrap() = query["code_challenge"].clone();
    *issuer.nonce.lock().unwrap() = query["nonce"].clone();
    let mut headers = HeaderMap::new();
    headers.insert(
        "cookie",
        HeaderValue::from_str(cookie.to_str().unwrap().split(';').next().unwrap()).unwrap(),
    );
    (headers, query["state"].clone())
}
#[tokio::test]
async fn oidc_sign_in_binds_pkce_nonce_session_project_and_csrf() {
    let (auth, issuer, server) = fixture().await;
    let (flow_headers, state) = flow(&auth, &issuer);
    assert_eq!(
        auth.callback(&HeaderMap::new(), &state, "code".into())
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    let cookie = auth
        .callback(&flow_headers, &state, "code".into())
        .await
        .unwrap();
    assert!(cookie.to_str().unwrap().contains("HttpOnly; SameSite=Lax"));
    assert_eq!(
        auth.callback(&flow_headers, &state, "code".into())
            .await
            .unwrap_err(),
        CloudError::PermissionDenied
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        "cookie",
        HeaderValue::from_str(cookie.to_str().unwrap().split(';').next().unwrap()).unwrap(),
    );
    assert_eq!(
        auth.caller(&headers, "other-project", false).unwrap_err(),
        CloudError::PermissionDenied
    );
    assert_eq!(
        auth.caller(&headers, "project-a", true).unwrap_err(),
        CloudError::PermissionDenied
    );
    headers.insert("origin", HeaderValue::from_static("http://127.0.0.1:5180"));
    headers.insert("x-colossus-csrf", HeaderValue::from_static("1"));
    assert_eq!(
        auth.caller(&headers, "project-a", true).unwrap().subject(),
        "alice"
    );
    auth.logout(&headers).unwrap();
    assert_eq!(
        auth.caller(&headers, "project-a", false).unwrap_err(),
        CloudError::PermissionDenied
    );
    server.abort();
}
#[tokio::test]
async fn invalid_oidc_tokens_never_create_sessions() {
    let (auth, issuer, server) = fixture().await;
    for invalid in [
        "nonce",
        "audience",
        "issuer",
        "expiry",
        "subject",
        "access-hash",
        "signature",
    ] {
        *issuer.invalid.lock().unwrap() = invalid;
        let (headers, state) = flow(&auth, &issuer);
        assert_eq!(
            auth.callback(&headers, &state, "code".into())
                .await
                .unwrap_err(),
            CloudError::PermissionDenied,
            "{invalid}"
        );
    }
    assert!(auth.sessions.lock().unwrap().is_empty());
    server.abort();
}
