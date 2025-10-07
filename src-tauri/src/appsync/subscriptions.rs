use super::worker::WsWrite;
use crate::twitch::{register_streamers_webhook, Broadcaster};
use futures_util::SinkExt;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tokio_tungstenite::tungstenite::protocol::Message;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ActiveSubscription {
  pub query: String,
  pub variables: Value,
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

pub async fn generate_desired_subscriptions(
  streamer_ids: &[String],
) -> HashMap<String, ActiveSubscription> {
  let broadcasters: Vec<Broadcaster> = streamer_ids
    .iter()
    .filter_map(|id| match id.parse::<u64>() {
      Ok(parsed_id) => Some(Broadcaster {
        broadcaster_id: parsed_id,
      }),
      Err(e) => {
        eprintln!("Skipping invalid broadcaster id '{}': {}", id, e);
        None
      }
    })
    .collect();

  if !broadcasters.is_empty() {
    register_streamers_webhook(broadcasters).await;
  }

  streamer_ids
    .iter()
    .map(|bid| {
      let uuid = Uuid::new_v4().to_string();
      let query = subscription_query();
      let vars = json!({ "broadcaster_id": bid.clone() });
      let sub = ActiveSubscription {
        query,
        variables: vars,
      };
      (uuid, sub)
    })
    .collect()
}

pub async fn manage_subscriptions(
  write: &mut WsWrite,
  token: &str,
  http_uri: &str,
  current_subs: &HashMap<String, ActiveSubscription>,
  desired_subs: &HashMap<String, ActiveSubscription>,
  pending_subscriptions: &mut HashSet<String>,
) -> anyhow::Result<()> {
  let current_ids: HashSet<_> = current_subs.keys().collect();
  let desired_ids: HashSet<_> = desired_subs.keys().collect();

  for sub_id in current_ids.difference(&desired_ids) {
    let stop_msg = json!({ "id": *sub_id, "type": "stop" }).to_string();
    write.send(Message::Text(stop_msg)).await?;
    pending_subscriptions.remove(*sub_id);
  }

  for sub_id in desired_ids.difference(&current_ids) {
    if let Some(sub) = desired_subs.get(*sub_id) {
      let start_msg = json!({
          "id": *sub_id,
          "type": "start",
          "payload": {
              "data": json!({
                  "query": &sub.query,
                  "variables": &sub.variables
              }).to_string(),
              "extensions": {
                  "authorization": {
                      "Authorization": format!("Bearer {}", token),
                      "host": http_uri
                  }
              }
          },
      })
      .to_string();

      write.send(Message::Text(start_msg)).await?;
      pending_subscriptions.insert((*sub_id).clone());
    }
  }

  Ok(())
}
