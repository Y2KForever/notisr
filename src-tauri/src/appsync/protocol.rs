use super::subscriptions::{
  generate_desired_subscriptions, manage_subscriptions,
};
use super::util;
use super::worker::{AppSyncWorker, WsWrite};
use crate::notifications::send_notification;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tauri::Emitter;

#[derive(Deserialize, Debug)]
struct IncomingMessage<'a> {
  id: Option<&'a str>,
  #[serde(rename = "type")]
  msg_type: &'a str,
  payload: Option<Value>,
}

#[derive(Serialize, Debug, Clone)]
struct StreamerUpdateEvent {
  sub_id: Option<String>,
  broadcaster_id: Option<String>,
  payload: Value,
}

pub async fn handle_message(
  worker: &mut AppSyncWorker,
  write: &mut WsWrite,
  text: &str,
) -> anyhow::Result<bool> {
  let msg: IncomingMessage = match serde_json::from_str(text) {
    Ok(m) => m,
    Err(e) => {
      eprintln!("Failed to parse incoming JSON: {}. Message: '{}'", e, text);
      return Ok(true);
    }
  };

  match msg.msg_type {
    "connection_ack" => handle_connection_ack(worker, write).await?,
    "ka" | "keepalive" => { /* Keepalive received, no action needed */ }
    "start_ack" => handle_start_ack(worker, msg.id),
    "data" | "next" => handle_data(worker, msg.id, msg.payload),
    "error" | "connection_error" => {
      return handle_error(worker, msg.payload).await
    }
    "complete" => handle_complete(worker, msg.id),
    _ => println!("Received unknown message type: {}", msg.msg_type),
  }

  Ok(true)
}

pub async fn update_and_manage_subscriptions(
  worker: &mut AppSyncWorker,
  write: &mut WsWrite,
  streamer_ids: Vec<String>,
) -> anyhow::Result<()> {
  let desired_subs = generate_desired_subscriptions(&streamer_ids).await;

  if worker.is_connected {
    let token = worker.token.read().await.clone();
    manage_subscriptions(
      write,
      &token,
      &worker.http_uri,
      &worker.active_subscriptions,
      &desired_subs,
      &mut worker.pending_subscriptions,
    )
    .await?;
  }

  worker.active_subscriptions = desired_subs;
  Ok(())
}

async fn handle_connection_ack(
  worker: &mut AppSyncWorker,
  write: &mut WsWrite,
) -> anyhow::Result<()> {
  println!("Connection acknowledged by server.");
  worker.is_connected = true;

  let token = worker.token.read().await.clone();
  let desired_subs = worker.active_subscriptions.clone();

  manage_subscriptions(
    write,
    &token,
    &worker.http_uri,
    &HashMap::new(),
    &desired_subs,
    &mut worker.pending_subscriptions,
  )
  .await?;

  Ok(())
}

fn handle_start_ack(worker: &mut AppSyncWorker, id: Option<&str>) {
  if let Some(id_str) = id {
    if worker.pending_subscriptions.remove(id_str) {
      println!("Subscription acknowledged: {}", id_str);
    }
  }
}

fn handle_data(
  worker: &mut AppSyncWorker,
  id: Option<&str>,
  payload: Option<Value>,
) {
  let payload = payload.unwrap_or(Value::Null);
  let streamer_obj = payload
    .get("data")
    .and_then(|d| d.get("onUpdateStreamer"))
    .or_else(|| payload.get("onUpdateStreamer"))
    .cloned()
    .unwrap_or(Value::Null);

  let event_payload = StreamerUpdateEvent {
    sub_id: id.map(String::from),
    broadcaster_id: streamer_obj
      .get("broadcaster_id")
      .and_then(Value::as_str)
      .map(String::from),
    payload: streamer_obj.clone(),
  };

  if let Err(e) = worker.app_handle.emit("streamer:update", event_payload) {
    eprintln!("Error emitting 'streamer:update' event: {}", e);
  }

  if let (Some(name), Some(update_type)) = (
    streamer_obj.get("broadcaster_name").and_then(Value::as_str),
    streamer_obj.get("type").and_then(Value::as_str),
  ) {
    let title = streamer_obj
      .get("title")
      .and_then(Value::as_str)
      .unwrap_or("");
    let category = streamer_obj
      .get("category")
      .and_then(Value::as_str)
      .unwrap_or("");
    let msg = format!("{} - {}", category, title);

    let heading = match update_type {
      "channel_updated" => format!("{} - Channel updated", name),
      "status" => format!("{} just went live!", name),
      _ => return,
    };

    let _ = send_notification(
      heading,
      msg,
      name.to_string(),
      worker.app_handle.clone(),
    );
  }
}

async fn handle_error(
  worker: &mut AppSyncWorker,
  payload: Option<Value>,
) -> anyhow::Result<bool> {
  eprintln!("Received error from server: {:?}", payload);
  let is_auth_error = payload
    .as_ref()
    .and_then(|p| p.get("errors"))
    .and_then(|e| e.as_array()?.get(0)?.get("message")?.as_str())
    .map(|s| s.to_lowercase().contains("unauthor"))
    .unwrap_or(false);

  if is_auth_error {
    println!("Authorization error detected. Attempting to refresh token.");
    if let Ok(Some(refresh_token)) =
      util::load_secret_blocking("refresh_token".to_string()).await
    {
      if let Ok(new_token) =
        util::refresh_access_token_blocking(refresh_token).await
      {
        println!("Token refreshed successfully.");
        *worker.token.write().await = new_token;
        return Ok(false);
      }
    }
  }

  Ok(false)
}

fn handle_complete(worker: &mut AppSyncWorker, id: Option<&str>) {
  if let Some(id_str) = id {
    println!("Subscription complete: {}", id_str);
    worker.pending_subscriptions.remove(id_str);
  }
}
