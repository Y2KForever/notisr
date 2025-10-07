mod protocol;
mod subscriptions;
mod util;
mod worker;

use std::sync::{Mutex, OnceLock};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use worker::AppSyncWorker;

#[derive(Debug)]
pub enum ControlMsg {
  UpdateSubscriptions { streamer_ids: Vec<String> },

  Stop,
}

static CTRL_SENDER: OnceLock<Mutex<Option<UnboundedSender<ControlMsg>>>> =
  OnceLock::new();

pub fn start_ws_client(
  app_handle: tauri::AppHandle,
  token: String,
) -> Result<(), String> {
  let sender_cell = CTRL_SENDER.get_or_init(|| Mutex::new(None));
  let mut guard = sender_cell.lock().unwrap();

  if guard.is_some() {
    return Err("Client is already running.".into());
  }

  let (tx, rx) = unbounded_channel();
  *guard = Some(tx);

  tauri::async_runtime::spawn(async move {
    let worker = AppSyncWorker::new(app_handle, rx, token).await;
    if let Err(e) = worker.run().await {
      eprintln!("AppSync worker exited with an error: {}", e);
    }
  });

  Ok(())
}

pub fn stop_ws_client() -> Result<(), String> {
  let sender_cell = CTRL_SENDER.get().ok_or("Client is not running.")?;
  let guard = sender_cell.lock().unwrap();
  let sender = guard.as_ref().ok_or("Client is not running.")?;

  sender
    .send(ControlMsg::Stop)
    .map_err(|e| format!("Failed to send stop signal: {}", e))
}
