use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use dotenvy_macro::dotenv;
use keyring::Entry;
use rand::RngCore;
use reqwest::blocking::Client as BlockingClient;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
  error::Error,
  io::{Error as IoError, ErrorKind},
};

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
pub struct ValidateResp {
  pub expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct RefreshResp {
  access_token: String,
  refresh_token: String,
}

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
  let client_id = dotenv!("CLIENT_ID");
  let client_secret = dotenv!("CLIENT_SECRET");

  let params = [
    ("client_id", client_id),
    ("client_secret", client_secret),
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
