use crate::oauth::refresh_access_token;

pub async fn refresh_access_token_blocking(
  refresh_token: String,
) -> anyhow::Result<String> {
  tokio::task::spawn_blocking(move || {
    refresh_access_token(&refresh_token)
      .map_err(|e| anyhow::anyhow!("Failed to refresh access token: {:?}", e))
  })
  .await?
}

pub async fn load_secret_blocking(
  key: String,
) -> anyhow::Result<Option<String>> {
  tokio::task::spawn_blocking(move || crate::util::load_secret(&key))
    .await
    .map_err(|e| anyhow::anyhow!("Task for secret loading failed: {}", e))
}
