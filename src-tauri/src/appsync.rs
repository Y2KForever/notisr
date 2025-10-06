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
  AddSub {
    sub_id: String,
    query: String,
    variables: Value,
  },
  RemoveSub {
    sub_id: String,
  },
  Stop,
}

#[derive(Debug)]
#[allow(unused)]
enum Action {
  Start {
    sub_id: String,
    query: String,
    variables: Value,
  },
  Stop {
    sub_id: String,
  },
}

static CTRL_SENDER: OnceLock<Mutex<Option<UnboundedSender<ControlMsg>>>> =
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
async fn sync_followed_streamers_on_connect(
  token: &Arc<RwLock<String>>,
  user_id: &str,
  subs: &mut HashMap<String, (String, Value)>,
  broadcaster_to_uuid: &mut HashMap<String, String>,
  pending_subscriptions: &mut HashSet<String>,
) -> Result<(), String> {
  let current_token = {
    let tk = token.read().await;
    tk.clone()
  };

  match fetch_followed_streamers(&current_token, &user_id).await {
    Ok(new_ids_vec) => {
      let new_ids: HashSet<String> = new_ids_vec.into_iter().collect();
      let current_ids: HashSet<String> =
        broadcaster_to_uuid.keys().cloned().collect();

      let added: Vec<String> =
        new_ids.difference(&current_ids).cloned().collect();
      let removed: Vec<String> =
        current_ids.difference(&new_ids).cloned().collect();

      let mut broadcasters: Vec<Broadcaster> = Vec::new();
      let mut valid_added: Vec<String> = Vec::new();

      for s in &added {
        match s.parse::<u64>() {
          Ok(id) => {
            broadcasters.push(Broadcaster { broadcaster_id: id });
            valid_added.push(s.clone());
          }
          Err(e) => {
            eprintln!("Skipping invalid broadcaster id '{}': {}", s, e);
          }
        }
      }

      if !broadcasters.is_empty() {
        register_streamers_webhook(broadcasters).await;
      }

      for bid in valid_added {
        let uuid = Uuid::new_v4().to_string();
        let query = subscription_query();
        let vars = json!({ "broadcaster_id": bid.clone() });
        subs.insert(uuid.clone(), (query.clone(), vars.clone()));
        broadcaster_to_uuid.insert(bid.clone(), uuid.clone());
      }

      for bid in removed {
        if let Some(uuid) = broadcaster_to_uuid.remove(&bid) {
          subs.remove(&uuid);
          pending_subscriptions.remove(&uuid);
        }
      }

      Ok(())
    }
    Err(e) => Err(format!(
      "Failed fetch_followed_streamers on reconnect: {}",
      e
    )),
  }
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

  let mut subs: HashMap<String, (String, Value)> = HashMap::new();
  let mut broadcaster_to_uuid: HashMap<String, String> = HashMap::new();
  let mut pending_subscriptions: HashSet<String> = HashSet::new();

  #[allow(unused)]
  let (action_tx, mut action_rx) = unbounded_channel::<Action>();

  let user_id = match load_secret_blocking("user_id".to_string()).await {
    Some(user_id) => user_id,
    None => {
      println!("User ID not found");
      "".to_string()
    }
  };

  match {
    let initial_token = {
      let tk = token.read().await;
      tk.clone()
    };
    fetch_followed_streamers(&initial_token, &user_id).await
  } {
    Ok(ids) => {
      for bid in ids {
        let uuid = Uuid::new_v4().to_string();
        let query = subscription_query();
        let vars = json!({"broadcaster_id": bid.clone()});
        subs.insert(uuid.clone(), (query, vars));
        broadcaster_to_uuid.insert(bid, uuid);
      }
    }
    Err(e) => {
      eprintln!("Failed fetch_followed_streamers: {}", e);
    }
  }

  let window = app_handle
    .get_webview_window("main")
    .ok_or_else(|| "no main window".to_string())?;

  let mut reload_interval = tokio::time::interval(Duration::from_secs(180));
  reload_interval
    .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

  let mut backoff_attempt: u32 = 0;

  loop {
    let cur_token = token.read().await;
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
          if let Some(refresh_token) =
            load_secret_blocking("refresh_token".to_string()).await
          {
            match tokio::time::timeout(
              std::time::Duration::from_secs(10),
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
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
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
          eprintln!(
            "Handshake OK: status={} headers={:#?}",
            resp.status(),
            resp.headers()
          );
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
                        Some(ControlMsg::AddSub { sub_id, query, variables}) => {
                            let uuid = Uuid::new_v4().to_string();
                            subs.insert(uuid.clone(), (query.clone(), variables.clone()));
                            broadcaster_to_uuid.insert(sub_id, uuid.clone());

                            if connected {
                                let data_obj = json!({
                                    "query": query,
                                    "variables": variables
                                });
                                let data_str = serde_json::to_string(&data_obj).unwrap();
                                let start = json!({
                                    "id": uuid,
                                    "type": "start",
                                    "payload": {
                                        "data": data_str,
                                        "extensions": {
                                            "authorization": {
                                                "Authorization": format!("Bearer {}", *cur_token),
                                                "host": http_uri
                                            }
                                        }
                                    },
                                }).to_string();
                                pending_subscriptions.insert(uuid.clone());
                                let _ = write.send(Message::Text(start)).await;
                            }
                        }
                        Some(ControlMsg::RemoveSub { sub_id }) => {
                            if let Some(uuid) = broadcaster_to_uuid.remove(&sub_id) {
                                subs.remove(&uuid);
                                pending_subscriptions.remove(&uuid);

                                if connected {
                                    let stop = serde_json::json!({ "id": uuid, "type": "stop" }).to_string();
                                    let _ = write.send(Message::Text(stop)).await;
                                }
                            }
                        }
                        Some(ControlMsg::Stop) | None => {
                            let _ = write.send(Message::Close(None)).await;
                            return Ok(());
                        }
                    }
                }

                 _ = reload_interval.tick() => {
                    println!("New tick");
                    let current_token = { let tk = token.read().await; tk.clone() };
                    match fetch_followed_streamers(&current_token, &user_id).await {
                      Ok(new_ids_vec) => {
                        let new_ids: HashSet<String> = new_ids_vec.into_iter().collect();
                        let current_ids: HashSet<String> = broadcaster_to_uuid.keys().cloned().collect();
                        let added: Vec<String> = new_ids.difference(&current_ids).cloned().collect();
                        let mut broadcasters: Vec<Broadcaster> = Vec::new();
                        let mut valid_added: Vec<String> = Vec::new();

                        for s in &added {
                            match s.parse::<u64>() {
                                Ok(id) => {
                                    broadcasters.push(Broadcaster { broadcaster_id: id });
                                    valid_added.push(s.clone());
                                }
                                Err(e) => {
                                    eprintln!("Skipping invalid broadcaster id '{}': {}", s, e);
                                }
                            }
                        }

                        if !broadcasters.is_empty() {
                          register_streamers_webhook(broadcasters).await;
                        }
                        for bid in valid_added {
                            let uuid = Uuid::new_v4().to_string();
                            let query = subscription_query();
                            let vars = json!({ "broadcaster_id": bid.clone() });
                            subs.insert(uuid.clone(), (query.clone(), vars.clone()));
                            broadcaster_to_uuid.insert(bid.clone(), uuid.clone());

                            if connected {
                                let data_obj = json!({
                                    "query": query,
                                    "variables": vars
                                });
                                let data_str = serde_json::to_string(&data_obj).unwrap();
                                let start = json!({
                                    "id": uuid,
                                    "type": "start",
                                    "payload": {
                                        "data": data_str,
                                        "extensions": {
                                            "authorization": {
                                                "Authorization": format!("Bearer {}", current_token),
                                                "host": http_uri
                                            }
                                        }
                                    },
                                }).to_string();
                                pending_subscriptions.insert(uuid.clone());
                                let _ = write.send(Message::Text(start)).await;
                            }
                        }
                        for bid in current_ids.difference(&new_ids).cloned().collect::<Vec<_>>() {
                            if let Some(uuid) = broadcaster_to_uuid.remove(&bid) {
                                subs.remove(&uuid);
                                pending_subscriptions.remove(&uuid);

                                if connected {
                                    let stop = serde_json::json!({ "id": uuid, "type": "stop" }).to_string();
                                    let _ = write.send(Message::Text(stop)).await;
                                }
                            }
                        }
                    }
                        Err(e) => {
                            eprintln!("Failed reload fetch_followed_streamers: {}", e);
                        }
                    }
                }

                action = action_rx.recv() => {
                    match action {
                        Some(Action::Start { sub_id, query, variables }) => {
                            if connected {
                                if let Some(uuid) = broadcaster_to_uuid.get(&sub_id) {
                                    let data_obj = json!({
                                        "query": query,
                                        "variables": variables
                                    });
                                    let data_str = serde_json::to_string(&data_obj).unwrap();
                                    let start = json!({
                                        "id": uuid,
                                        "type": "start",
                                        "payload": {
                                            "data": data_str,
                                            "extensions": {
                                                "authorization": {
                                                    "Authorization": format!("Bearer {}", *cur_token),
                                                    "host": http_uri
                                                }
                                            }
                                        },
                                    }).to_string();
                                    pending_subscriptions.insert(uuid.clone());
                                    let _ = write.send(Message::Text(start)).await;
                                }
                            }
                        }
                        Some(Action::Stop { sub_id }) => {
                            if connected {
                                if let Some(uuid) = broadcaster_to_uuid.get(&sub_id) {
                                    let stop = serde_json::json!({ "id": uuid, "type": "stop" }).to_string();
                                    pending_subscriptions.remove(uuid);
                                    let _ = write.send(Message::Text(stop)).await;
                                }
                            }
                        }
                        None => {}
                    }
                }

                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(txt))) => {
                            if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                                if let Some(t) = v.get("type").and_then(|t| t.as_str()) {
                                    println!("Type: {:?}", t);
                                    match t {
                                        "connection_ack" => {
                                            println!("Connection_ack received");
                                            connected = true;

                                            if let Err(e) = sync_followed_streamers_on_connect(&token, &user_id, &mut subs, &mut broadcaster_to_uuid, &mut pending_subscriptions).await {
                                                eprintln!("{}", e);
                                            }
                                            let cur_token_for_start = { let tk = token.read().await; tk.clone() };
                                            for (uuid, (q, vars)) in subs.iter() {
                                                let data_obj = json!({
                                                    "query": q,
                                                    "variables": vars
                                                });
                                                let data_str = serde_json::to_string(&data_obj).unwrap();
                                                let start = json!({
                                                    "id": uuid,
                                                    "type": "start",
                                                    "payload": {
                                                        "data": data_str,
                                                        "extensions": {
                                                            "authorization": {
                                                                "Authorization": format!("Bearer {}", cur_token_for_start),
                                                                "host": http_uri
                                                            }
                                                        }
                                                    },
                                                }).to_string();
                                                pending_subscriptions.insert(uuid.clone());
                                                let _ = write.send(Message::Text(start)).await;
                                            }
                                        }
                                        "ka" | "keepalive" => {
                                            println!("keepalive recieved");
                                          }
                                        "start_ack" => {
                                            println!("start_ack recieved");
                                            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                                                println!("start_ack received for subscription: {}", id);
                                                pending_subscriptions.remove(id);
                                            }
                                        }
                                        "data" | "next" => {
                                            println!("data recieved");
                                            let payload_val = v.get("payload").cloned().unwrap_or(Value::Null);
                                            println!("{:?}", payload_val);
                                            let streamer_obj = payload_val
                                                .get("data")
                                                .and_then(|d| d.get("onUpdateStreamer"))
                                                .cloned()
                                                .or_else(|| payload_val.get("onUpdateStreamer").cloned())
                                                .unwrap_or(Value::Null);

                                            let maybe_id = v.get("id").and_then(|i| i.as_str()).map(|s| s.to_string());
                                            let mut out = serde_json::Map::new();
                                            if let Some(id) = maybe_id { out.insert("sub_id".into(), Value::String(id)); }
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

                                            let want_refresh = v.get("payload").
                                            and_then(|p| p.get("errors")).
                                            and_then(|errs| errs.as_array()).
                                            and_then(|arr| arr.get(0)).
                                            and_then(|first| first.get("message")).
                                            and_then(|m| m.as_str()).map(|s| s.to_lowercase().contains("unauthor") ).
                                            unwrap_or(false);

                                            if want_refresh {
                                                match load_secret_blocking("refresh_token".to_string()).await {
                                                    Some(refresh_token)=>{
                                                        match refresh_access_token_blocking(refresh_token).await {
                                                            Ok(new_token) => {
                                                                eprintln!("Token refreshed");
                                                                {
                                                                    let mut tk=token.write().await;
                                                                    *tk=new_token.clone();
                                                                }
                                                                let _ =write.send(Message::Close(None)).await;
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
                                        "message" => {
                                            println!("Wtf?");
                                        }
                                        "complete" => {
                                            if let Some(id) = v.get("id").and_then(|i| i.as_str()) {
                                                pending_subscriptions.remove(id);
                                            }
                                            println!("Complete recieved.")
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
                        Some(Err(e)) => { eprintln!("ws read err: {}", e); break; }
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
