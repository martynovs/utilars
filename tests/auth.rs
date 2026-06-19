mod common;

use common::{ACCOUNT, TEST_KEY};
use utilars::SignerSource;

#[tokio::test]
async fn jwt_has_the_required_claims() {
    use base64::Engine;
    use utilars::TokenManager;

    #[derive(serde::Deserialize)]
    struct Claims {
        sub: String,
        aud: String,
        exp: u64,
    }

    let tokens = TokenManager::new(ACCOUNT, SignerSource::local_pem(TEST_KEY).unwrap());
    let token = tokens.token().await.unwrap();
    let payload_b64 = token.split('.').nth(1).expect("payload");
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .unwrap();
    let claims: Claims = serde_json::from_slice(&payload).unwrap();

    assert_eq!(claims.sub, ACCOUNT);
    assert_eq!(claims.aud, "https://api.utila.io/");
    assert!(claims.exp > 0);
}
