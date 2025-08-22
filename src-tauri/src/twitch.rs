use reqwest::Client;
use serde_json::Value;

pub async fn fetch_followed_streamers(
  token: &str,
  user_id: &str,
) -> Result<Vec<String>, String> {
  let client_id = std::env::var("CLIENT_ID")
    .map_err(|_| "CLIENT_ID env not set".to_string())?;

  let client = Client::builder()
    .build()
    .map_err(|e| format!("reqwest build: {}", e))?;

  let mut after: Option<String> = None;
  let mut collected: Vec<String> = Vec::new();

  loop {
    let mut url = format!(
      "https://api.twitch.tv/helix/channels/followed?user_id={}&first=100",
      user_id
    );

    if let Some(cursor) = &after {
      url.push_str(&format!("&after={}", cursor));
    }

    let resp = client
      .get(&url)
      .header("Client-Id", client_id.clone())
      .header("Authorization", format!("Bearer {}", token))
      .send()
      .await
      .map_err(|e| format!("twitch request err: {}", e))?;

    if !resp.status().is_success() {
      let status = resp.status();
      let body = resp.text().await.unwrap_or_default();
      return Err(format!("twitch API error {}: {}", status, body));
    }

    let body: Value = resp
      .json()
      .await
      .map_err(|e| format!("json parse: {}", e))?;
    if let Some(arr) = body.get("data").and_then(|d| d.as_array()) {
      for item in arr {
        if let Some(to_id) = item.get("broadcaster_id").and_then(|v| v.as_str())
        {
          collected.push(to_id.to_string());
        }
      }
    }

    after = body
      .get("pagination")
      .and_then(|p| p.get("cursor"))
      .and_then(|c| c.as_str())
      .map(|s| s.to_string());

    if after.is_none() {
      break;
    }
  }

  collected.sort();
  collected.dedup();
  Ok(collected)
}
