use keyring::Entry;
use tauri::AppHandle;

use crate::{appsync::start_ws_client, twitch::register_streamers_webhook};

pub fn load_secret(name: &str) -> Option<String> {
  Entry::new("notisr", name)
    .ok()?
    .get_secret()
    .ok()
    .and_then(|bytes| String::from_utf8(bytes).ok())
}

pub fn spawn_new_user(access_token: String, user: String, app: AppHandle, token_ws: String) {
  tauri::async_runtime::spawn(async move {
    register_streamers_webhook(access_token, user).await;

    if let Err(e) = start_ws_client(app, token_ws) {
      eprintln!("start_ws_client failed after registering webhook: {:?}", e)
    }
  });
}