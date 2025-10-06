use std::{
  collections::HashSet,
  sync::{mpsc::Sender, Arc, Mutex, OnceLock},
  thread::JoinHandle,
};

use crate::{
  appsync::ControlMsg,
  handle_setup_user,
  oauth::{gen_b64_url, generate_pkce_pair},
  twitch::fetch_followed_streamers,
  util::load_secret,
};
use dotenvy_macro::dotenv;
use once_cell::sync::OnceCell;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::mpsc::UnboundedSender;
use url::Url;

#[derive(Serialize, Deserialize, Debug)]
pub struct Broadcasters {
  pub broadcaster_id: String,
  pub broadcaster_name: String,
  pub category: String,
  pub title: String,
  pub is_live: bool,
  pub profile_picture: Option<String>,
}

pub struct ServerCtl {
  pub stop_tx: Sender<()>,
  pub handle: JoinHandle<()>,
}

static CTRL_SENDER: OnceCell<UnboundedSender<ControlMsg>> = OnceCell::new();

static CURRENT_SUBSCRIPTIONS: OnceLock<Mutex<HashSet<String>>> =
  OnceLock::new();

#[tauri::command]
pub fn shutdown_server(
  state: tauri::State<std::sync::Mutex<Option<ServerCtl>>>,
) {
  let mut guard = state.inner().lock().unwrap();
  if let Some(ctl) = guard.take() {
    let _ = ctl.stop_tx.send(());
    std::thread::spawn(move || {
      let _ = ctl.handle.join();
    });
  }
}

#[tauri::command]
pub fn on_startup(
  state: tauri::State<'_, Mutex<Option<String>>>,
) -> Option<String> {
  let current_state = state.lock().unwrap();
  if current_state.is_none() {
    Some("log_in".to_string())
  } else {
    // fetch streamers from ddb.
    None
  }
}

#[tauri::command]
pub fn login(app: AppHandle) {
  let client_id = dotenv!("CLIENT_ID");
  let redirect_uri = dotenv!("REDIRECT_URI");
  let scope = dotenv!("SCOPE");

  let (pkce_challenge, pkce_verifier) = generate_pkce_pair();
  let csrf_state = gen_b64_url();
  let nonce = gen_b64_url();

  let verifier_arc = Arc::new(Mutex::new(Some(pkce_verifier)));

  let mut auth_url = Url::parse("https://id.twitch.tv/oauth2/authorize")
    .expect("valid base url");

  auth_url
    .query_pairs_mut()
    .append_pair("force_verify", "true")
    .append_pair("response_type", "code")
    .append_pair("client_id", &client_id)
    .append_pair("redirect_uri", &redirect_uri)
    .append_pair("scope", &scope)
    .append_pair("state", &csrf_state)
    .append_pair("code_challenge", &pkce_challenge)
    .append_pair("code_challenge_method", "S256")
    .append_pair("nonce", &nonce);

  let url_string = auth_url.clone();

  let ctl = handle_setup_user(app.clone(), csrf_state, verifier_arc.clone());
  app.manage(std::sync::Mutex::new(Some(ctl)));

  let _ = app.opener().open_url(url_string, None::<&str>);
}

#[tauri::command]
pub fn add_subscription(broadcaster_id: String) -> Result<(), String> {
  let sender = CTRL_SENDER
    .get()
    .ok_or_else(|| "client not running".to_string())?;

  let current_subs =
    CURRENT_SUBSCRIPTIONS.get_or_init(|| Mutex::new(HashSet::new()));

  let mut subs = current_subs.lock().unwrap();
  subs.insert(broadcaster_id.clone());

  let streamer_ids: Vec<String> = subs.iter().cloned().collect();

  sender
    .send(ControlMsg::UpdateSubscriptions { streamer_ids })
    .map_err(|e| format!("send error: {}", e))?;

  Ok(())
}

#[tauri::command]
pub fn remove_subscription(broadcaster_id: String) -> Result<(), String> {
  let sender = CTRL_SENDER
    .get()
    .ok_or_else(|| "client not running".to_string())?;

  let current_subs =
    CURRENT_SUBSCRIPTIONS.get_or_init(|| Mutex::new(HashSet::new()));

  let mut subs = current_subs.lock().unwrap();
  subs.remove(&broadcaster_id);

  let streamer_ids: Vec<String> = subs.iter().cloned().collect();

  sender
    .send(ControlMsg::UpdateSubscriptions { streamer_ids })
    .map_err(|e| format!("send error: {}", e))?;

  Ok(())
}

#[tauri::command]
pub fn fetch_streamers(app: AppHandle) {
  let base_uri = dotenv!("BASE_URI");
  let token = load_secret("access_token").unwrap_or_default();
  let user_id = load_secret("user_id").unwrap_or_default();

  if token.is_empty() || user_id.is_empty() {
    eprintln!("Missing token or user_id");
    return;
  }

  tauri::async_runtime::spawn(async move {
    let broadcaster_ids = match fetch_followed_streamers(&token, &user_id).await
    {
      Ok(ids) => ids,
      Err(e) => {
        eprintln!("Failed to fetch followed streamers: {:?}", e);
        return;
      }
    };

    let client = Client::new();

    let res = client
      .post(format!("{}/streamers/fetch-all", base_uri))
      .json(&broadcaster_ids)
      .send()
      .await;

    let streamers = match res {
      Ok(resp) => match resp.json::<Vec<Broadcasters>>().await {
        Ok(s) => s,
        Err(e) => {
          eprintln!("Failed to deserialize JSON: {:?}", e);
          return;
        }
      },
      Err(e) => {
        eprintln!("Failed to fetch streamers: {:?}", e);
        return;
      }
    };

    let (mut live, mut offline): (Vec<Broadcasters>, Vec<Broadcasters>) =
      streamers.into_iter().partition(|b| b.is_live);

    live.sort_by(|a, b| {
      a.broadcaster_name
        .to_lowercase()
        .cmp(&b.broadcaster_name.to_lowercase())
    });
    offline.sort_by(|a, b| {
      a.broadcaster_name
        .to_lowercase()
        .cmp(&b.broadcaster_name.to_lowercase())
    });

    app
      .emit(
        "streamers:fetched",
        json!({"online": live, "offline": offline}),
      )
      .unwrap_or_else(|e| eprintln!("Failed to emit event: {:?}", e));
  });
}

#[tauri::command]
pub fn open_broadcaster_url(app: AppHandle, broadcaster_name: String) {
  println!("Broadcaster: {:?}", broadcaster_name);
  let _ = app.opener().open_url(
    format!("https://twitch.tv/{}", broadcaster_name),
    None::<&str>,
  );
}
