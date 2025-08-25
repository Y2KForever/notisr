use keyring::Entry;
use tauri::AppHandle;

use crate::{
  appsync::start_ws_client,
  oauth::{refresh_access_token, validate_access_token},
  twitch::register_streamers_webhook,
};

pub fn load_secret(name: &str) -> Option<String> {
  Entry::new("notisr", name)
    .ok()?
    .get_secret()
    .ok()
    .and_then(|bytes| String::from_utf8(bytes).ok())
}

pub fn spawn_new_user(
  access_token: String,
  user: String,
  token_ws: String,
  app: AppHandle,
) {
  tauri::async_runtime::spawn(async move {
    register_streamers_webhook(access_token, user).await;

    if let Err(e) = start_ws_client(app, token_ws) {
      eprintln!("start_ws_client failed after registering webhook: {:?}", e)
    }
  });
}

pub fn check_validitiy_token() -> Option<String> {
  return match load_secret("access_token") {
    Some(existing_token) => match validate_access_token(&existing_token) {
      Ok(Some(_resp)) => Some(existing_token),
      Ok(None) => {
        eprintln!(
          "Access token invalid (401). Attempting refresh if possible..."
        );
        if let Some(refresh_token) = load_secret("refresh_token") {
          match refresh_access_token(&refresh_token) {
            Ok(_) => match load_secret("access_token") {
              Some(new_access) => {
                eprintln!("Token refresh succeeded; starting WS client with refreshed token");
                Some(new_access)
              }
              None => {
                eprintln!("Token refresh succeeded but new access token not found in keyring");
                None
              }
            },
            Err(err) => {
              eprintln!("Token refresh failed: {:?}", err);
              None
            }
          }
        } else {
          eprintln!("No refresh token available to refresh access token");
          None
        }
      }
      Err(err) => {
        eprintln!(
          "validate_access_token returned error during startup: {:?}",
          err
        );
        None
      }
    },
    None => {
      eprintln!("No access token found in keyring; WS client will not start");
      None
    }
  };
}
