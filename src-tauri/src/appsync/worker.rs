use super::protocol::{handle_message, update_and_manage_subscriptions};
use super::subscriptions::{self, ActiveSubscription};
use super::util;
use super::ControlMsg;
use crate::twitch::fetch_followed_streamers;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use dotenvy_macro::dotenv;
use futures_util::{stream::SplitSink, SinkExt, StreamExt};
use http::Request;
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::RwLock;
use tokio_tungstenite::tungstenite::handshake::client::generate_key;
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use url::Url;

pub type WsWrite =
  SplitSink<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>, Message>;

pub struct AppSyncWorker {
  pub app_handle: AppHandle,
  pub ctrl_rx: UnboundedReceiver<ControlMsg>,
  pub token: Arc<RwLock<String>>,
  pub http_uri: String,
  realtime_uri: String,
  user_id: String,
  pub active_subscriptions: HashMap<String, ActiveSubscription>,
  pub pending_subscriptions: HashSet<String>,
  pub is_connected: bool,
}

impl AppSyncWorker {
  pub async fn new(
    app_handle: AppHandle,
    ctrl_rx: UnboundedReceiver<ControlMsg>,
    token: String,
  ) -> Self {
    let user_id = util::load_secret_blocking("user_id".to_string())
      .await
      .ok()
      .flatten()
      .unwrap_or_default();

    let initial_streamers = fetch_followed_streamers(&token, &user_id)
      .await
      .unwrap_or_else(|e| {
        eprintln!("Failed to fetch initial streamers: {}", e);
        Vec::new()
      });

    let active_subscriptions =
      subscriptions::generate_desired_subscriptions(&initial_streamers).await;

    Self {
      app_handle,
      ctrl_rx,
      token: Arc::new(RwLock::new(token)),
      http_uri: dotenv!("APPSYNC_HTTP_URI").to_string(),
      realtime_uri: dotenv!("APPSYNC_REALTIME_URI").to_string(),
      user_id,
      active_subscriptions,
      pending_subscriptions: HashSet::new(),
      is_connected: false,
    }
  }

  pub async fn run(mut self) -> anyhow::Result<()> {
    println!("AppSync worker starting.");
    let mut backoff_attempt: u32 = 0;
    let mut reload_interval = tokio::time::interval(Duration::from_secs(180));
    reload_interval
      .set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    'reconnect_loop: loop {
      println!("Attempting to connect to AppSync...");
      match self.connect().await {
        Ok(ws_stream) => {
          println!("WebSocket connection established.");
          backoff_attempt = 0;
          self.is_connected = false;

          let (mut write, mut read) = ws_stream.split();

          let init_msg =
            serde_json::json!({ "type": "connection_init" }).to_string();
          if write.send(Message::Text(init_msg)).await.is_err() {
            eprintln!("Failed to send connection_init, reconnecting.");
            continue 'reconnect_loop;
          }

          'message_loop: loop {
            tokio::select! {
                Some(msg) = self.ctrl_rx.recv() => {
                    match msg {
                        ControlMsg::Stop => {
                            println!("Stop signal received. Shutting down worker.");
                            let _ = write.send(Message::Close(None)).await;
                            return Ok(());
                        }
                        ControlMsg::UpdateSubscriptions { streamer_ids } => {
                            println!("Received request to update subscriptions.");
                            if let Err(e) = update_and_manage_subscriptions(&mut self, &mut write, streamer_ids).await {
                                eprintln!("Failed to update subscriptions: {}", e);
                            }
                        }
                    }
                }

                _ = reload_interval.tick() => {
                    println!("Periodically reloading followed streamers.");
                    let token = self.token.read().await.clone();
                    match fetch_followed_streamers(&token, &self.user_id).await {
                        Ok(streamer_ids) => {
                            if let Err(e) = update_and_manage_subscriptions(&mut self, &mut write, streamer_ids).await {
                                eprintln!("Failed to update subscriptions after reload: {}", e);
                            }
                        }
                        Err(e) => eprintln!("Failed to fetch followed streamers: {}", e),
                    }
                }

                Some(msg_result) = read.next() => {
                    match msg_result {
                        Ok(Message::Text(text)) => {
                            if !handle_message(&mut self, &mut write, &text).await? {
                                break 'message_loop;
                            }
                        }
                        Ok(Message::Close(_)) => {
                            println!("Received Close frame. Reconnecting.");
                            break 'message_loop;
                        }
                        Err(e) => {
                            eprintln!("WebSocket read error: {}. Reconnecting.", e);
                            break 'message_loop;
                        }
                        _ => {}
                    }
                }
            }
          }
        }
        Err(e) => {
          eprintln!("Connection failed: {}. Attempting to refresh token.", e);
          // Attempt to refresh token if connection fails, as it might be expired
          if let Ok(Some(refresh_token)) =
            util::load_secret_blocking("refresh_token".to_string()).await
          {
            if let Ok(new_token) =
              util::refresh_access_token_blocking(refresh_token).await
            {
              *self.token.write().await = new_token;
              println!("Refreshed token due to connection failure.");
            }
          }
        }
      }

      backoff_attempt = backoff_attempt.saturating_add(1);
      let power = std::cmp::min(backoff_attempt, 6);
      let base_delay_ms = 1000.0 * (2.0f64.powi(power as i32));
      let jitter = rand::rng().random_range(0.5..1.5);
      let backoff_duration =
        Duration::from_millis((base_delay_ms * jitter) as u64);

      println!(
        "Reconnecting in {:.2} seconds...",
        backoff_duration.as_secs_f64()
      );
      tokio::time::sleep(backoff_duration).await;
    }
  }

  async fn connect(
    &self,
  ) -> anyhow::Result<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>
  {
    let token = self.token.read().await;
    let header_json = serde_json::json!({
        "host": self.http_uri,
        "Authorization": format!("Bearer {}", *token)
    });
    let header_b64 = URL_SAFE_NO_PAD.encode(header_json.to_string().as_bytes());
    let header_sub = format!("header-{}", header_b64);
    let protocols_value = format!("graphql-ws,{}", header_sub);

    let url_str = format!("wss://{}/graphql", self.realtime_uri);
    let url = Url::parse(&url_str)?;
    let host = url
      .host_str()
      .ok_or_else(|| anyhow::anyhow!("URL has no host"))?;

    let request = Request::builder()
      .method("GET")
      .uri(&url_str)
      .header("Host", host)
      .header("Connection", "Upgrade")
      .header("Upgrade", "websocket")
      .header("Sec-WebSocket-Version", "13")
      .header("Sec-WebSocket-Protocol", protocols_value)
      .header("Sec-WebSocket-Key", generate_key())
      .body(())?;

    let (ws_stream, response) = connect_async(request).await?;

    if response.status().as_u16() != 101 {
      return Err(anyhow::anyhow!(
        "WebSocket handshake failed with status: {}",
        response.status()
      ));
    }

    Ok(ws_stream)
  }
}
