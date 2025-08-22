use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use keyring::Entry;
use once_cell::sync::Lazy;
use rand::RngCore;
use reqwest::blocking::Client as BlockingClient;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use std::{
  collections::HashMap,
  time::{Duration, Instant},
};
use std::{
  error::Error,
  io::{Error as IoError, ErrorKind},
};

#[derive(Debug, Deserialize)]
pub struct Claims {
  iss: String,
  pub sub: String,
  aud: Value,
  nonce: Option<String>,
}

struct CachedJwks {
  jwks: Value,
  fetched_at: Instant,
}

#[derive(Deserialize, Debug)]
pub struct ValidateResp {
  pub expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct RefreshResp {
  access_token: String,
  refresh_token: String,
}

static JWKS_CACHE: Lazy<Mutex<HashMap<String, CachedJwks>>> =
  Lazy::new(|| Mutex::new(HashMap::new()));

const JWKS_TTL: Duration = Duration::from_secs(60 * 60);

const CLOCK_SKEW_LEEWAY_SECS: u64 = 60;

fn random_base64url(len_bytes: usize) -> String {
  let mut buf = vec![0u8; len_bytes];
  rand::rng().fill_bytes(&mut buf);
  URL_SAFE_NO_PAD.encode(buf)
}

pub fn gen_b64_url() -> String {
  random_base64url(32)
}

pub fn generate_pkce_pair() -> (String, String) {
  let verifier = random_base64url(32);
  let digest = Sha256::digest(verifier.as_bytes());
  let challenge = URL_SAFE_NO_PAD.encode(digest);
  (challenge, verifier)
}

fn fetch_jwks_cached(jwks_uri: &str) -> Result<Value, Box<dyn Error>> {
  {
    let cache = JWKS_CACHE.lock().unwrap();
    if let Some(entry) = cache.get(jwks_uri) {
      if entry.fetched_at.elapsed() < JWKS_TTL {
        return Ok(entry.jwks.clone());
      }
    }
  }

  let client = BlockingClient::new();
  let resp_text = client
    .get(jwks_uri)
    .send()
    .map_err(|e| Box::new(e) as Box<dyn Error>)?
    .error_for_status()
    .map_err(|e| Box::new(e) as Box<dyn Error>)?
    .text()
    .map_err(|e| Box::new(e) as Box<dyn Error>)?;

  let jwks: Value = serde_json::from_str(&resp_text)
    .map_err(|e| Box::new(e) as Box<dyn Error>)?;

  {
    let mut cache = JWKS_CACHE.lock().unwrap();
    cache.insert(
      jwks_uri.to_string(),
      CachedJwks {
        jwks: jwks.clone(),
        fetched_at: Instant::now(),
      },
    );
  }

  Ok(jwks)
}

fn find_jwk_for_kid<'a>(jwks: &'a Value, kid: &'a str) -> Option<&'a Value> {
  let keys = jwks.get("keys")?.as_array()?;
  keys
    .iter()
    .find(|k| k.get("kid").unwrap().as_str().unwrap() == kid)
}

fn aud_matches(aud_value: &Value, expected_aud: &str) -> bool {
  match aud_value {
    Value::String(s) => s == expected_aud,
    Value::Array(arr) => arr
      .iter()
      .any(|v| v.as_str().map_or(false, |s| s == expected_aud)),
    _ => false,
  }
}

pub fn verify_id_token(
  id_token: &str,
  jwks_uri: &str,
  expected_issuer: &str,
  expected_aud: &str,
  expected_nonce: Option<&str>,
) -> Result<Claims, Box<dyn Error>> {
  let header =
    decode_header(id_token).map_err(|e| Box::new(e) as Box<dyn Error>)?;
  let kid = header.kid.as_deref().ok_or_else(|| {
    Box::new(IoError::new(
      ErrorKind::Other,
      "id_token missing 'kid' header",
    )) as Box<dyn Error>
  })?;
  let alg = header.alg;
  if alg != Algorithm::RS256 {
    return Err(Box::new(IoError::new(
      ErrorKind::Other,
      format!("unexpected token alg: {:?}", alg),
    )));
  }

  let jwks = fetch_jwks_cached(jwks_uri)?;
  let jwk = find_jwk_for_kid(&jwks, kid).ok_or_else(|| {
    Box::new(IoError::new(
      ErrorKind::Other,
      format!("no jwk with kid {}", kid),
    )) as Box<dyn Error>
  })?;

  let n = jwk.get("n").and_then(|v| v.as_str()).ok_or_else(|| {
    Box::new(IoError::new(ErrorKind::Other, "JWK missing 'n'"))
      as Box<dyn Error>
  })?;
  let e = jwk.get("e").and_then(|v| v.as_str()).ok_or_else(|| {
    Box::new(IoError::new(ErrorKind::Other, "JWK missing 'e'"))
      as Box<dyn Error>
  })?;

  let decoding_key = DecodingKey::from_rsa_components(n, e)
    .map_err(|e| Box::new(e) as Box<dyn Error>)?;

  let mut validation = Validation::new(Algorithm::RS256);
  validation.leeway = CLOCK_SKEW_LEEWAY_SECS;
  validation.validate_exp = true;

  let token_data = decode::<Claims>(id_token, &decoding_key, &validation)
    .map_err(|e| Box::new(e) as Box<dyn Error>)?;
  let claims = token_data.claims;

  if claims.iss != expected_issuer {
    return Err(Box::new(IoError::new(
      ErrorKind::Other,
      format!("issuer mismatch: {} != {}", claims.iss, expected_issuer),
    )));
  }

  if !aud_matches(&claims.aud, expected_aud) {
    return Err(Box::new(IoError::new(
      ErrorKind::Other,
      "audience did not contain expected audience",
    )));
  }

  if let Some(expected_nonce) = expected_nonce {
    match &claims.nonce {
      Some(nonce) if nonce == expected_nonce => {}
      _ => {
        return Err(Box::new(IoError::new(ErrorKind::Other, "nonce mismatch")))
      }
    }
  }

  Ok(claims)
}

pub fn validate_access_token(
  access_token: &str,
) -> Result<Option<ValidateResp>, Box<dyn Error>> {
  let client = BlockingClient::new();
  let resp = client
    .get("https://id.twitch.tv/oauth2/validate")
    .header("Authorization", format!("OAuth {}", access_token))
    .send()?;

  if resp.status().is_success() {
    let v: ValidateResp = resp.json()?;
    Ok(Some(v))
  } else if resp.status().as_u16() == 401 {
    Ok(None)
  } else {
    Err(Box::new(IoError::new(
      ErrorKind::Other,
      format!("validate returned HTTP {}", resp.status()),
    )))
  }
}

pub fn refresh_access_token(
  refresh_token: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync + 'static>> {
  dotenvy::dotenv().ok();
  let client_id = match std::env::var("CLIENT_ID") {
    Ok(v) => v,
    Err(e) => {
      eprintln!("CLIENT_ID not set: {:?}", e);
      return Err(Box::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!("CLIENT_ID not set"),
      )));
    }
  };
  let client_secret = match std::env::var("CLIENT_SECRET") {
    Ok(v) => v,
    Err(e) => {
      eprintln!("CLIENT_SECRET not set: {:?}", e);
      return Err(Box::new(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!("CLIENT_SECRET not set"),
      )));
    }
  };

  let params = [
    ("client_id", client_id.as_str()),
    ("client_secret", client_secret.as_str()),
    ("grant_type", "refresh_token"),
    ("refresh_token", refresh_token),
  ];

  let client = BlockingClient::new();
  let resp = client
    .post("https://id.twitch.tv/oauth2/token")
    .form(&params)
    .send()
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
    .unwrap();

  let status = resp.status();
  let body = resp
    .text()
    .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
    .unwrap();

  if status.is_success() {
    let raw: RefreshResp = serde_json::from_str(&body)
      .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
      .unwrap();

    Entry::new("notisr", "access_token")
      .and_then(|e| e.set_secret(raw.access_token.as_bytes()))
      .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
      .unwrap();

    Entry::new("notisr", "refresh_token")
      .and_then(|e| e.set_secret(raw.refresh_token.as_bytes()))
      .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)
      .unwrap();

    return Ok(raw.refresh_token);
  }

  if status.as_u16() == 401 {
    let _ =
      Entry::new("notisr", "access_token").and_then(|e| e.delete_credential());
    let _ =
      Entry::new("notisr", "refresh_token").and_then(|e| e.delete_credential());
    return Err(Box::new(std::io::Error::new(
      std::io::ErrorKind::PermissionDenied,
      format!("refresh token invalid (401): {}", body),
    )));
  }

  Err(Box::new(std::io::Error::new(
    std::io::ErrorKind::Other,
    format!("refresh failed: {} body: {}", status, body),
  )))
}
