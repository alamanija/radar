use super::*;
use jsonwebtoken::{encode, DecodingKey, EncodingKey, Header};
use rsa::pkcs1::{EncodeRsaPrivateKey, EncodeRsaPublicKey, LineEnding};
use serde_json::{json, Value};
use std::sync::OnceLock;

const ISS: &str = "https://app.clerk.accounts.dev";
const OAUTH_ISS: &str = "https://oauth.clerk.accounts.dev";
const CLIENT: &str = "client_abc";

/// (private PEM, public PEM), generated once per test run.
fn keys() -> &'static (String, String) {
    static K: OnceLock<(String, String)> = OnceLock::new();
    K.get_or_init(|| {
        let sk = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
        let pk = sk.to_public_key();
        (
            sk.to_pkcs1_pem(LineEnding::LF).unwrap().to_string(),
            pk.to_pkcs1_pem(LineEnding::LF).unwrap(),
        )
    })
}

fn jwks() -> std::sync::Arc<JwksCache> {
    JwksCache::with_keys(vec![(
        "k1".into(),
        DecodingKey::from_rsa_pem(keys().1.as_bytes()).unwrap(),
    )])
}

fn sign(mut claims: Value) -> String {
    claims["sub"] = json!("user_1");
    claims["exp"] = json!(chrono::Utc::now().timestamp() + 600);
    let mut h = Header::new(Algorithm::RS256);
    h.kid = Some("k1".into());
    encode(
        &h,
        &claims,
        &EncodingKey::from_rsa_pem(keys().0.as_bytes()).unwrap(),
    )
    .unwrap()
}

fn cfg(client: Option<&str>, oauth_iss: bool) -> VerifyConfig {
    let mut issuers = vec![ISS.to_string()];
    if oauth_iss {
        issuers.push(OAUTH_ISS.to_string());
    }
    VerifyConfig {
        issuers,
        oauth_client_id: client.map(Into::into),
    }
}

#[tokio::test]
async fn accepts_oauth_jwt_with_matching_client_id() {
    let t = sign(json!({"iss": ISS, "client_id": CLIENT, "scope": "openid profile"}));
    let u = verify(&t, &jwks(), &cfg(Some(CLIENT), false)).await.unwrap();
    assert_eq!(u.id, "user_1");
}

#[tokio::test]
async fn rejects_oauth_jwt_with_other_client_id() {
    let t = sign(json!({"iss": ISS, "client_id": "other", "scope": "openid"}));
    assert!(verify(&t, &jwks(), &cfg(Some(CLIENT), false)).await.is_err());
    // azp-only OAuth token (has scope) with the wrong value is rejected too.
    let t = sign(json!({"iss": ISS, "azp": "other", "scope": "openid"}));
    assert!(verify(&t, &jwks(), &cfg(Some(CLIENT), false)).await.is_err());
}

#[tokio::test]
async fn accepts_session_jwt_without_client_id() {
    let t = sign(json!({"iss": ISS, "azp": "http://localhost:1420"}));
    let u = verify(&t, &jwks(), &cfg(Some(CLIENT), false)).await.unwrap();
    assert_eq!(u.id, "user_1");
}

#[tokio::test]
async fn rejects_wrong_issuer() {
    let t = sign(json!({"iss": "https://evil.example", "client_id": CLIENT, "scope": "openid"}));
    assert!(verify(&t, &jwks(), &cfg(Some(CLIENT), true)).await.is_err());
}

#[tokio::test]
async fn accepts_oauth_issuer_when_configured() {
    let t = sign(json!({"iss": OAUTH_ISS, "client_id": CLIENT, "scope": "openid"}));
    assert!(verify(&t, &jwks(), &cfg(Some(CLIENT), true)).await.is_ok());
    assert!(verify(&t, &jwks(), &cfg(Some(CLIENT), false)).await.is_err());
}
