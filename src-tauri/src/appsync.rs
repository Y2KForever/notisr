use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use dotenvy_macro::dotenv;
use futures_util::{SinkExt, StreamExt};
use http::Request;
use rand::Rng;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::{sync::Arc, time::Duration};
use tauri::{Emitter, Manager};
use tokio::sync::mpsc::{
  unbounded_channel, UnboundedReceiver, UnboundedSender,
};
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use url::Url;
use uuid::Uuid;

use crate::notifications::send_notification;
use crate::oauth::refresh_access_token;
use crate::twitch::{
  fetch_followed_streamers, register_streamers_webhook, Broadcaster,
};

#[derive(Debug)]
pub enum ControlMsg {
  UpdateSubscriptions { streamer_ids: Vec<String> },
  Stop,
}

#[derive(Debug, Clone)]
struct ActiveSubscription {
  query: String,
  variables: Value,
}

static CTRL_SENDER: OnceLock<Mutex<Option<UnboundedSender<ControlMsg>>>> =
  OnceLock::new();

static CURRENT_SUBSCRIPTIONS: OnceLock<Mutex<HashSet<String>>> =
  OnceLock::new();

async fn refresh_access_token_blocking(
  refresh_token: String,
) -> Result<String, String> {
  tokio::task::spawn_blocking(move || {
    match refresh_access_token(&refresh_token) {
      Ok(tok) => Ok(tok),
      Err(e) => Err(format!("refresh_access_token error: {:?}", e)),
    }
  })
  .await
  .map_err(|e| format!("spawn_blocking join error: {:?}", e))?
}

async fn load_secret_blocking(key: String) -> Option<String> {
  tokio::task::spawn_blocking(move || crate::util::load_secret(&key))
    .await
    .ok()
    .flatten()
}

async fn manage_subscriptions(
  write: &mut futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<
      tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    Message,
  >,
  token: &str,
  http_uri: &str,
  current_subs: &HashMap<String, ActiveSubscription>,
  desired_subs: &HashMap<String, ActiveSubscription>,
  pending_subscriptions: &mut HashSet<String>,
) -> Result<(), String> {
  let current_ids: HashSet<&String> = current_subs.keys().collect();
  let desired_ids: HashSet<&String> = desired_subs.keys().collect();

  for sub_id in current_ids.difference(&desired_ids) {
    let stop_msg = json!({
      "id": *sub_id,
      "type": "stop"
    })
    .to_string();
    let _ = write.send(Message::Text(stop_msg)).await;
    pending_subscriptions.remove(*sub_id);
  }

  for sub_id in desired_ids.difference(&current_ids) {
    if let Some(sub) = desired_subs.get(*sub_id) {
      let data_obj = json!({
        "query": &sub.query,
        "variables": &sub.variables
      });
      let data_str = serde_json::to_string(&data_obj).unwrap();

      let start_msg = json!({
        "id": *sub_id,
        "type": "start",
        "payload": {
          "data": data_str,
          "extensions": {
            "authorization": {
              "Authorization": format!("Bearer {}", token),
              "host": http_uri
            }
          }
        },
      })
      .to_string();

      pending_subscriptions.insert((*sub_id).clone());
      let _ = write.send(Message::Text(start_msg)).await;
    }
  }

  Ok(())
}

pub fn start_ws_client(
  app_handle: tauri::AppHandle,
  token: String,
) -> Result<(), String> {
  let sender_cell = CTRL_SENDER.get_or_init(|| Mutex::new(None));
  let mut guard = sender_cell.lock().unwrap();

  if guard.is_some() {
    return Err("Client already running".into());
  }

  let (tx, rx) = unbounded_channel();
  *guard = Some(tx.clone());

  let token: Arc<RwLock<String>> = Arc::new(RwLock::new(token));

  tauri::async_runtime::spawn(worker_loop(app_handle, rx, token));
  Ok(())
}

pub fn stop_ws_client() -> Result<(), String> {
  let sender_cell = CTRL_SENDER
    .get()
    .ok_or_else(|| "client not running".to_string())?;

  let guard = sender_cell.lock().unwrap();
  let sender = guard
    .as_ref()
    .ok_or_else(|| "client not running".to_string())?;

  sender
    .send(ControlMsg::Stop)
    .map_err(|e| format!("send error: {}", e))?;

  Ok(())
}

fn subscription_query() -> String {
  r#"subscription OnUpdateStreamer($broadcaster_id: String!) {
        onUpdateStreamer(broadcaster_id: $broadcaster_id) {
            broadcaster_id
            broadcaster_name
            category
            title
            is_live
            type
        }
    }"#
    .to_string()
}

async fn update_subscriptions_internal(
  streamer_ids: Vec<String>,
  active_subscriptions: &mut HashMap<String, ActiveSubscription>,
  connected: bool,
  write: &mut futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<
      tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    Message,
  >,
  token: &Arc<RwLock<String>>,
  http_uri: &str,
  pending_subscriptions: &mut HashSet<String>,
) {
  let mut desired_subs = HashMap::new();
  let mut broadcasters: Vec<Broadcaster> = Vec::new();

  for bid in &streamer_ids {
    match bid.parse::<u64>() {
      Ok(id) => {
        broadcasters.push(Broadcaster { broadcaster_id: id });
      }
      Err(e) => {
        eprintln!("Skipping invalid broadcaster id '{}': {}", bid, e);
        continue;
      }
    }
  }

  if !broadcasters.is_empty() {
    register_streamers_webhook(broadcasters).await;
  }

  for bid in streamer_ids {
    let uuid = Uuid::new_v4().to_string();
    let query = subscription_query();
    let vars = json!({"broadcaster_id": bid.clone()});
    desired_subs.insert(
      uuid,
      ActiveSubscription {
        query,
        variables: vars,
      },
    );
  }

  if connected {
    let token_str = token.read().await.clone();
    if let Err(e) = manage_subscriptions(
      write,
      &token_str,
      http_uri,
      &*active_subscriptions,
      &desired_subs,
      pending_subscriptions,
    )
    .await
    {
      eprintln!("Failed to update subscriptions: {}", e);
    }
  }

  *active_subscriptions = desired_subs;
}

async fn worker_loop(
  app_handle: tauri::AppHandle,
  mut ctrl_rx: UnboundedReceiver<ControlMsg>,
  token: Arc<RwLock<String>>,
) -> Result<(), String> {
  println!("Worker loop started");

  let http_uri = dotenv!("APPSYNC_HTTP_URI");
  let realtime_uri = dotenv!("APPSYNC_REALTIME_URI");
  let ws_path = "/graphql";
  let appsync_proto = "graphql-ws";

  let mut active_subscriptions: HashMap<String, ActiveSubscription> =
    HashMap::new();
  let mut pending_subscriptions: HashSet<String> = HashSet::new();

  let user_id = match load_secret_blocking("user_id".to_string()).await {
    Some(user_id) => user_id,
    None => {
      println!("User ID not found");
      "".to_string()
    }
  };

  let initial_streamers = {
    let initial_token = { token.read().await.clone() };
    match fetch_followed_streamers(&initial_token, &user_id).await {
      Ok(streamers) => streamers,
      Err(e) => {
        eprintln!("Failed initial fetch_followed_streamers: {}", e);
        Vec::new()
      }
    }
  };

  {
    let current_subs =
      CURRENT_SUBSCRIPTIONS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut subs = current_subs.lock().unwrap();
    *subs = initial_streamers.iter().cloned().collect();
  }

  let mut broadcasters: Vec<Broadcaster> = Vec::new();
  for bid in &initial_streamers {
    match bid.parse::<u64>() {
      Ok(id) => {
        broadcasters.push(Broadcaster { broadcaster_id: id });
      }
      Err(e) => {
        eprintln!("Skipping invalid broadcaster id '{}': {}", bid, e);
      }
    }
  }

  if !broadcasters.is_empty() {
    register_streamers_webhook(broadcasters).await;
  }

  for bid in initial_streamers {
    let uuid = Uuid::new_v4().to_string();
    let query = subscription_query();
    let vars = json!({"broadcaster_id": bid.clone()});
    active_subscriptions.insert(
      uuid,
      ActiveSubscription {
        query,
        variables: vars,
      },
    );
  }

  let window = app_handle
    .get_webview_window("main")
    .ok_or_else(|| "no main window".to_string())?;

  let mut reload_interval = tokio::time::interval(Duration::from_secs(180));
  reload_interval
    .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

  let mut backoff_attempt: u32 = 0;

  loop {
    let cur_token = token.read().await.clone();
    let header_json = serde_json::json!({
        "host": http_uri,
        "Authorization": format!("Bearer {}", cur_token)
    })
    .to_string();
    let header_b64 = URL_SAFE_NO_PAD.encode(header_json.as_bytes());
    let header_sub = format!("header-{}", header_b64);

    let protocols_value = format!("{}, {}", appsync_proto, header_sub);

    let ws_url = format!("wss://{}{}", realtime_uri, ws_path);
    let url = Url::parse(&ws_url).map_err(|e| e.to_string())?;
    let req = Request::builder()
      .method("GET")
      .uri(url.as_str())
      .header("Host", url.host_str().unwrap())
      .header("Connection", "upgrade")
      .header("Upgrade", "websocket")
      .header("sec-websocket-version", "13")
      .header("sec-websocket-protocol", protocols_value.as_str())
      .header("sec-websocket-key", generate_key())
      .body(())
      .map_err(|e| e.to_string())?;

    match connect_async(req).await {
      Ok((ws_stream, resp)) => {
        if !(resp.status().is_success() || resp.status().as_u16() == 101) {
          // Handle connection failure with token refresh
          if let Some(refresh_token) =
            load_secret_blocking("refresh_token".to_string()).await
          {
            match tokio::time::timeout(
              Duration::from_secs(10),
              refresh_access_token_blocking(refresh_token),
            )
            .await
            {
              Ok(Ok(new_token)) => {
                eprintln!("refreshed token OK");
                {
                  let mut tk = token.write().await;
                  *tk = new_token.clone();
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
              }
              Ok(Err(err)) => {
                eprintln!("refresh returned Err: {}", err);
              }
              Err(_) => {
                eprintln!("refresh_access_token timed out after 10s");
              }
            }
          } else {
            eprintln!("no refresh token available");
          }
          eprintln!("websocket handshake not 101; will backoff and retry");
        } else {
          eprintln!("Handshake OK: status={}", resp.status());
          backoff_attempt = 0;

          let (mut write, mut read) = ws_stream.split();

          let init =
            serde_json::json!({ "type": "connection_init", "payload": {} })
              .to_string();
          let _ = write.send(Message::Text(init)).await;

          println!("Connection init sent");

          let mut connected = false;

          loop {
            tokio::select! {
                ctrl = ctrl_rx.recv() => {
                    match ctrl {
                        Some(ControlMsg::UpdateSubscriptions { streamer_ids }) => {
                            // Update the global subscription tracker
                            {
                                let current_subs = CURRENT_SUBSCRIPTIONS.get_or_init(|| Mutex::new(HashSet::new()));
                                let mut subs = current_subs.lock().unwrap();
                                *subs = streamer_ids.iter().cloned().collect();
                            }

                            update_subscriptions_internal(
                                streamer_ids,
                                &mut active_subscriptions,
                                connected,
                                &mut write,
                                &token,
                                http_uri,
                                &mut pending_subscriptions
                            ).await;
                        }
                        Some(ControlMsg::Stop) | None => {
                            let _ = write.send(Message::Close(None)).await;
                            return Ok(());
                        }
                    }
                }

                _ = reload_interval.tick() => {
                    println!("Reloading followed streamers");
                    let current_token = token.read().await.clone();
                    match fetch_followed_streamers(&current_token, &user_id).await {
                        Ok(new_ids) => {
                            // Update the global subscription tracker with the new Twitch follows
                            {
                                let current_subs = CURRENT_SUBSCRIPTIONS.get_or_init(|| Mutex::new(HashSet::new()));
                                let mut subs = current_subs.lock().unwrap();
                                *subs = new_ids.iter().cloned().collect();
                            }

                            update_subscriptions_internal(
                                new_ids,
                                &mut active_subscriptions,
                                connected,
                                &mut write,
                                &token,
                                http_uri,
                                &mut pending_subscriptions
                            ).await;
                        }
                        Err(e) => {
                            eprintln!("Failed to reload followed streamers: {}", e);
                        }
                    }
                }

                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(txt))) => {
                            if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                                if let Some(t) = v.get("type").and_then(|t| t.as_str()) {
                                    match t {
                                        "connection_ack" => {
                                            println!("Connection_ack received");
                                            connected = true;

                                            // Subscribe to all streamers at once
                                            let token_str = token.read().await.clone();
                                            let desired_subs = active_subscriptions.clone();
                                            if let Err(e) = manage_subscriptions(
                                                &mut write,
                                                &token_str,
                                                http_uri,
                                                &HashMap::new(), // Start with empty to add all
                                                &desired_subs,
                                                &mut pending_subscriptions
                                            ).await {
                                                eprintln!("Failed initial subscriptions: {}", e);
                                            }
                                        }
                                        "ka" | "keepalive" => {
                                            println!("keepalive received");
                                        }
                                        "start_ack" => {
                                            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                                                println!("start_ack received for subscription: {}", id);
                                                pending_subscriptions.remove(id);
                                            }
                                        }
                                        "data" | "next" => {
                                            let payload_val = v.get("payload").cloned().unwrap_or(Value::Null);
                                            let streamer_obj = payload_val
                                                .get("data")
                                                .and_then(|d| d.get("onUpdateStreamer"))
                                                .cloned()
                                                .or_else(|| payload_val.get("onUpdateStreamer").cloned())
                                                .unwrap_or(Value::Null);

                                            let maybe_id = v.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
                                            let mut out = serde_json::Map::new();
                                            if let Some(id) = maybe_id {
                                                out.insert("sub_id".into(), Value::String(id));
                                            }
                                            if let Some(bid) = streamer_obj.get("broadcaster_id").and_then(|b| b.as_str()) {
                                                out.insert("broadcaster_id".into(), Value::String(bid.to_string()));
                                            }
                                            out.insert("payload".into(), streamer_obj.clone());

                                            match window.emit("streamer:update", Value::Object(out.clone())) {
                                                Ok(_) => {
                                                    let name = match streamer_obj.get("broadcaster_name") {
                                                        Some(value) => match value {
                                                            serde_json::Value::String(s) => s.clone(),
                                                            _ => "Unknown".to_string(),
                                                        },
                                                        None => "Unknown".to_string(),
                                                    };

                                                    let title = match streamer_obj.get("title"){
                                                        Some(title) => { title.to_string()}
                                                        None => {"".to_string()}
                                                    };

                                                    let cat = match streamer_obj.get("category"){
                                                        Some(cat) => { cat.to_string()}
                                                        None => {"".to_string()}
                                                    };

                                                    let msg = format!("{} - {}", cat, title);

                                                    if streamer_obj.get("type").is_some_and(|x| x == "channel_updated") {
                                                      let heading = format!("{} - Channel updated", name);
                                                      _ = send_notification(heading, msg, name, app_handle.clone());
                                                    } else if streamer_obj.get("type").is_some_and(|x| x == "status") {
                                                      let heading = format!("{} - Just went live!", name);
                                                      _ = send_notification(heading, msg, name, app_handle.clone());
                                                    }
                                                }
                                                Err(e) => {println!("Error trying to emit streamer:update. {:?}", e)}
                                            }
                                        }
                                        "error" | "connection_error" => {
                                            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                                                pending_subscriptions.remove(id);
                                            }

                                            let want_refresh = v.get("payload")
                                                .and_then(|p| p.get("errors"))
                                                .and_then(|errs| errs.as_array())
                                                .and_then(|arr| arr.get(0))
                                                .and_then(|first| first.get("message"))
                                                .and_then(|m| m.as_str())
                                                .map(|s| s.to_lowercase().contains("unauthor"))
                                                .unwrap_or(false);

                                            if want_refresh {
                                                match load_secret_blocking("refresh_token".to_string()).await {
                                                    Some(refresh_token) => {
                                                        match refresh_access_token_blocking(refresh_token).await {
                                                            Ok(new_token) => {
                                                                eprintln!("Token refreshed");
                                                                {
                                                                    let mut tk = token.write().await;
                                                                    *tk = new_token.clone();
                                                                }
                                                                let _ = write.send(Message::Close(None)).await;
                                                                break;
                                                            }
                                                            Err(e) => {
                                                                eprintln!("Failed to refresh token: {}", e);
                                                            }
                                                        }
                                                    }
                                                    None => println!("Refresh token not available"),
                                                }
                                            }
                                            eprintln!("server error: {}", txt);
                                            let _ = write.send(Message::Close(None)).await;
                                            break;
                                        }
                                        "complete" => {
                                            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                                                pending_subscriptions.remove(id);
                                            }
                                            println!("Complete received.")
                                        }
                                        _ => {
                                            println!("Unknown message type: {}", t)
                                        }
                                    }
                                }
                            } else {
                                eprintln!("invalid json ws message: {}", txt);
                            }
                        }
                        Some(Ok(Message::Close(_))) => { break; }
                        Some(Ok(_)) => {}
                        Some(Err(e)) => {
                            eprintln!("ws read err: {}", e);
                            break;
                        }
                        None => { break; }
                    }
                }
            }
          }
        }
      }
      Err(e) => {
        eprintln!("connect error: {}", e.to_string());
      }
    }

    backoff_attempt = backoff_attempt.saturating_add(1);
    let pow = std::cmp::min(backoff_attempt, 6);
    let base = 2u64.pow(pow);
    let jitter = rand::rng().random_range(0.5..1.5);
    let backoff = ((base as f64) * 1000.0 * jitter) as u64;
    tokio::time::sleep(Duration::from_millis(backoff)).await;
  }
}
