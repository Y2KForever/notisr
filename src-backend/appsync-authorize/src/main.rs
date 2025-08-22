use dashmap::DashMap;
use lambda_runtime::{Error, LambdaEvent, service_fn};
use once_cell::sync::OnceCell;
use reqwest::Client;
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
struct CacheEntry {
    value: Value,
    expires_at_ms: u128,
}

static MEM_CACHE: OnceCell<DashMap<String, CacheEntry>> = OnceCell::new();

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis()
}

fn normalize_token_value(s: &str) -> String {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("Bearer ") {
        rest.to_string()
    } else if let Some(rest) = s.strip_prefix("OAuth ") {
        rest.to_string()
    } else {
        s.to_string()
    }
}

fn extract_token(event: &Value) -> Option<String> {
    if let Some(tok) = event.get("authorizationToken").and_then(|v| v.as_str()) {
        return Some(tok.to_string());
    }

    if let Some(headers) = event.get("headers") {
        if let Some(h) = headers
            .get("Authorization")
            .or_else(|| headers.get("authorization"))
        {
            if let Some(s) = h.as_str() {
                return Some(s.to_string());
            }
        }
    }

    None
}

fn mem_cache_get(token: &str) -> Option<(Value, i64)> {
    let cache = MEM_CACHE.get_or_init(|| DashMap::new());
    if let Some(entry) = cache.get(token) {
        let now = now_ms();
        if entry.expires_at_ms > now {
            let ttl_s = ((entry.expires_at_ms - now) / 1000) as i64;
            return Some((entry.value.clone(), ttl_s));
        } else {
            cache.remove(token);
        }
    }
    None
}

fn mem_cache_put(token: &str, info: Value, ttl_seconds: i64) {
    let cache = MEM_CACHE.get_or_init(|| DashMap::new());
    let expires_at_ms = now_ms() + (ttl_seconds as u128 * 1000);
    let ent = CacheEntry {
        value: info,
        expires_at_ms,
    };
    cache.insert(token.to_string(), ent);
}

async fn validate_with_twitch(client: &Client, token: &str) -> Result<(Value, i64), String> {
    let res = client
        .get("https://id.twitch.tv/oauth2/validate")
        .header("Authorization", format!("OAuth {}", token))
        .send()
        .await
        .map_err(|e| format!("reqwest error: {}", e))?;

    if !res.status().is_success() {
        let body = res.text().await.unwrap_or_default();
        return Err(format!("twitch validate failed: {}", body));
    }

    let v: Value = res.json().await.map_err(|e| format!("json parse: {}", e))?;
    let ttl = v.get("expires_in").and_then(|e| e.as_i64()).unwrap_or(60);
    Ok((v, ttl))
}

fn allow_response(
    user_id: Option<&str>,
    login: Option<&str>,
    client_id: Option<&str>,
    ttl_seconds: i64,
) -> Value {
    let mut ctx = serde_json::Map::new();
    if let Some(id) = user_id {
        ctx.insert("userId".into(), Value::String(id.into()));
    }
    if let Some(l) = login {
        ctx.insert("login".into(), Value::String(l.into()));
    }
    if let Some(cid) = client_id {
        ctx.insert("clientId".into(), Value::String(cid.into()));
    }

    println!("Authorized response");

    json!({
        "isAuthorized": true,
        "resolverContext": Value::Object(ctx),
        "ttlOverride": ttl_seconds
    })
}

fn deny_response() -> Value {
    println!("Denied response");
    json!({ "isAuthorized": false })
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    MEM_CACHE.get_or_init(|| DashMap::new());
    let func = service_fn(authorizer_handler);
    let _ = lambda_runtime::run(func).await;
    Ok(())
}

async fn authorizer_handler(event: LambdaEvent<Value>) -> Result<Value, Error> {
    println!("Event: {:?}", event);
    let raw_auth = match extract_token(&event.payload) {
        Some(s) => s,
        None => {
            println!("No token?");
            return Ok(deny_response());
        }
    };

    let token = normalize_token_value(&raw_auth);

    if let Some((info, ttl)) = mem_cache_get(&token) {
        let user_id = info.get("user_id").and_then(|v| v.as_str());
        let login = info.get("login").and_then(|v| v.as_str());
        let client_id = info.get("client_id").and_then(|v| v.as_str());
        return Ok(allow_response(user_id, login, client_id, ttl));
    }

    let client = Client::builder()
        .user_agent("tauri-appsync-authorizer/1.0")
        .build()
        .map_err(|e| format!("reqwest build: {}", e))?;

    match validate_with_twitch(&client, &token).await {
        Ok((info, ttl)) => {
            let ttl_sec = if ttl <= 0 { 60 } else { ttl };
            mem_cache_put(&token, info.clone(), ttl_sec);

            let user_id = info.get("user_id").and_then(|v| v.as_str());
            let login = info.get("login").and_then(|v| v.as_str());
            let client_id = info.get("client_id").and_then(|v| v.as_str());
            Ok(allow_response(user_id, login, client_id, ttl_sec))
        }
        Err(e) => {
            println!("Twitch verification failed. error: {:?}", e);
            Ok(deny_response())
        }
    }
}
