use std::time::SystemTime;

use aws_config::meta::region::RegionProviderChain;
use aws_config::BehaviorVersion;
use aws_credential_types::provider::ProvideCredentials;
use aws_credential_types::Credentials;
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::Client as DynamoDbClient;
use aws_sdk_secretsmanager::Client as SecretsClient;
use aws_sigv4::{
    http_request::{sign, SignableBody, SignableRequest, SigningSettings},
    sign::v4,
};
use chrono::Utc;
use hmac::{Hmac, Mac};
use lambda_http::http::HeaderMap;
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;

const STREAMER_TABLE_ENV: &str = "STREAMER_TABLE";
const SECRET_ARN_ENV: &str = "SECRET_ARN";
const APPSYNC_API_HOST_ENV: &str = "APPSYNC_API_HOST";

#[derive(Deserialize, Debug)]
struct TwitchWebhookEvent {
    subscription: Subscription,
    event: Value,
}

#[derive(Deserialize, Debug)]
struct Subscription {
    #[serde(rename = "type")]
    event_type: String,
}

#[derive(Deserialize, Debug)]
struct StreamEvent {
    broadcaster_user_id: String,
}

#[derive(Deserialize, Debug)]
struct ChannelUpdateEvent {
    broadcaster_user_id: String,
    title: String,
    category_name: String,
}

#[derive(Deserialize, Debug)]
struct ChallengePayload {
    challenge: String,
}
#[derive(Deserialize, Debug)]
struct TwitchSecretConfig {
    webhook_secret: String,
}

#[derive(Deserialize, Debug)]
struct GqlResponse {
    #[serde(default)]
    errors: Option<Vec<GqlError>>,
}

#[derive(Deserialize, Debug)]
struct GqlError {
    message: String,
}

async fn post_graphql_to_appsync(
    appsync_api_host: &str,
    region: &str,
    mutation: &str,
    variables: serde_json::Value,
) -> Result<(), String> {
    println!("Vars: {:?}", variables);
    let body_json = json!({
        "query": mutation,
        "variables": variables
    });
    let body_vec =
        serde_json::to_vec(&body_json).map_err(|e| format!("json stringify err: {}", e))?;

    let (access_key, secret_key, session_token) = get_runtime_aws_credentials().await?;

    let creds = Credentials::new(access_key, secret_key, session_token, None, "appsync");
    let identity = creds.into();
    let signing_settings = SigningSettings::default();
    let signing_params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name("appsync")
        .time(SystemTime::now())
        .settings(signing_settings)
        .build()
        .unwrap()
        .into();
    let url = format!("https://{}{}", appsync_api_host, "/graphql");

    let mut req: http::Request<Vec<u8>> = http::Request::builder()
        .method("POST")
        .uri(&url)
        .header("host", appsync_api_host)
        .header("content-type", "application/json")
        .body(body_vec.clone())
        .unwrap();

    let signable_request = SignableRequest::new(
        req.method().as_str(),
        req.uri().to_string(),
        req.headers()
            .iter()
            .map(|(k, v)| (k.as_str(), std::str::from_utf8(v.as_bytes()).unwrap())),
        SignableBody::Bytes(&body_vec),
    )
    .unwrap();

    let (signing_instructions, _signature) = sign(signable_request, &signing_params)
        .unwrap()
        .into_parts();

    signing_instructions.apply_to_request_http1x(&mut req);

    println!("Request: {:?}", req);

    let mut reqwest_headers = reqwest::header::HeaderMap::new();
    for (k, v) in req.headers().iter() {
        reqwest_headers.insert(k.clone(), v.clone());
    }

    let client = Client::new();
    let reqwest_req = client
        .request(req.method().clone(), req.uri().to_string())
        .headers(reqwest_headers)
        .body(body_vec)
        .build()
        .unwrap();

    let resp = client.execute(reqwest_req).await.unwrap();

    let status = resp.status().as_u16();
    let json = resp
        .json::<GqlResponse>()
        .await
        .map_err(|e| format!("failed to parse JSON response: {}", e))?;

    if let Some(errors) = json.errors {
        let joined = errors
            .iter()
            .map(|e| e.message.clone())
            .collect::<Vec<_>>()
            .join("; ");

        Err(format!("Appsync returned status {}: {}", status, joined))
    } else {
        Ok(())
    }
}

async fn get_runtime_aws_credentials() -> Result<(String, String, Option<String>), String> {
    let conf = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let provider = conf
        .credentials_provider()
        .ok_or_else(|| "no credentials provider available".to_string())?;
    let creds = provider
        .provide_credentials()
        .await
        .map_err(|e| format!("failed to fetch credentials: {}", e))?;

    let access_key = creds.access_key_id().to_string();
    let secret_key = creds.secret_access_key().to_string();
    let session_token = creds.session_token().map(|s| s.to_string());

    Ok((access_key, secret_key, session_token))
}

async fn get_twitch_secret_config(
    secrets_client: &SecretsClient,
) -> Result<TwitchSecretConfig, Box<dyn std::error::Error>> {
    let secret_arn =
        std::env::var(SECRET_ARN_ENV).expect("SECRET_ARN environment variable not set.");

    let resp = secrets_client
        .get_secret_value()
        .secret_id(secret_arn)
        .send()
        .await?;

    let secret_str = resp.secret_string.unwrap_or_default();
    let config: TwitchSecretConfig = serde_json::from_str(&secret_str)?;
    Ok(config)
}

fn is_valid_signature(headers: &HeaderMap, secret: &str, body: &str) -> bool {
    let msg_id = match headers
        .get("twitch-eventsub-message-id")
        .and_then(|v| v.to_str().ok())
    {
        Some(id) => id,
        None => {
            println!("DEBUG: missing message-id header");
            return false;
        }
    };
    let msg_ts = match headers
        .get("twitch-eventsub-message-timestamp")
        .and_then(|v| v.to_str().ok())
    {
        Some(ts) => ts,
        None => {
            println!("DEBUG: missing message-timestamp header");
            return false;
        }
    };
    let sig_header = match headers
        .get("twitch-eventsub-message-signature")
        .and_then(|v| v.to_str().ok())
    {
        Some(sig) => sig,
        None => {
            println!("DEBUG: missing signature header");
            return false;
        }
    };

    let message = format!("{}{}{}", msg_id, msg_ts, body);
    println!("DEBUG: HMAC input message = {}", message);

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC init failed");
    mac.update(message.as_bytes());
    let computed = mac.finalize().into_bytes();
    let computed_hex = hex::encode(computed);
    let computed_sig = format!("sha256={}", computed_hex);
    println!("DEBUG: computed signature = {}", computed_sig);

    let valid = sig_header.eq_ignore_ascii_case(&computed_sig);
    println!("DEBUG: signature valid = {}", valid);
    valid
}

async fn get_streamer_item(
    ddb_client: &DynamoDbClient,
    table_name: &str,
    broadcaster_id: &str,
) -> Result<Option<(String, String, String, bool, String)>, String> {
    let resp = ddb_client
        .get_item()
        .table_name(table_name)
        .key(
            "broadcaster_id",
            AttributeValue::S(broadcaster_id.to_string()),
        )
        .send()
        .await
        .map_err(|e| format!("DDB GetItem error: {}", e))?;

    if let Some(item) = resp.item {
        let name = item
            .get("broadcaster_name")
            .and_then(|v| v.as_s().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "".to_string());

        let title = item
            .get("title")
            .and_then(|v| v.as_s().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "".to_string());

        let category = item
            .get("category")
            .and_then(|v| v.as_s().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "".to_string());

        let is_live = item
            .get("is_live")
            .and_then(|v| v.as_bool().ok())
            .unwrap_or(&false)
            .clone();

        let updated = item
            .get("updated")
            .and_then(|v| v.as_s().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "".to_string());

        Ok(Some((name, title, category, is_live, updated)))
    } else {
        Ok(None)
    }
}

async fn function_handler(request: Request) -> Result<Response<Body>, Error> {
    let region_provider = RegionProviderChain::default_provider().or_else("eu-central-1");
    let region = region_provider.region().await.unwrap().to_string();
    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let secrets_client = SecretsClient::new(&config);
    let ddb_client = DynamoDbClient::new(&config);
    let appsync_api_host =
        std::env::var(APPSYNC_API_HOST_ENV).expect("APPSYNC_API_HOST environment variable not set");
    let table_name =
        std::env::var(STREAMER_TABLE_ENV).expect("STREAMER_TABLE environment variable not set");

    let secret = get_twitch_secret_config(&secrets_client)
        .await
        .expect("Failed to get Twitch secret");

    let body_str = match request.body() {
        Body::Text(s) => s.clone(),
        _ => String::new(),
    };

    if !is_valid_signature(request.headers(), &secret.webhook_secret, &body_str) {
        eprintln!("Signature validation failed");
        return Ok(Response::builder().status(403).body(Body::Empty).unwrap());
    }

    if let Ok(challenge) = serde_json::from_str::<ChallengePayload>(&body_str) {
        return Ok(Response::builder()
            .status(200)
            .header("Content-Type", "text/plain")
            .body(Body::Text(challenge.challenge))
            .unwrap());
    }

    let payload: TwitchWebhookEvent = match serde_json::from_str(&body_str) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Invalid payload JSON: {:?}", e);
            return Ok(Response::builder()
                .status(400)
                .body(Body::Text("Invalid JSON".into()))
                .unwrap());
        }
    };

    let variables: serde_json::Value = match payload.subscription.event_type.as_str() {
        "stream.online" | "stream.offline" => {
            let event: StreamEvent = serde_json::from_value(payload.event.clone()).unwrap();
            let is_live = payload.subscription.event_type == "stream.online";

            match get_streamer_item(&ddb_client, &table_name, &event.broadcaster_user_id).await {
                Ok(Some((_name, title, category, _old_live, updated))) => {
                    json!({
                        "broadcaster_id": event.broadcaster_user_id.clone(),
                        "category": category,
                        "title": title,
                        "is_live": is_live,
                        "updated": updated,
                        "type": if is_live { "status" } else {"offline"}
                    })
                }
                Ok(None) => {
                    json!({
                        "broadcaster_id": event.broadcaster_user_id.clone(),
                        "category": "",
                        "title": "",
                        "is_live": is_live,
                        "updated": Utc::now().to_rfc3339(),
                        "type": if is_live { "status" } else {"offline"}
                    })
                }
                Err(e) => {
                    eprintln!("Failed to read existing streamer before mutation: {}", e);
                    return Ok(Response::builder().status(500).body(Body::Empty).unwrap());
                }
            }
        }

        "channel.update" => {
            let event: ChannelUpdateEvent = serde_json::from_value(payload.event.clone()).unwrap();

            match get_streamer_item(&ddb_client, &table_name, &event.broadcaster_user_id).await {
                Ok(Some((_name, _title, _category, existing_live, updated))) => {
                    json!({
                        "broadcaster_id": event.broadcaster_user_id.clone(),
                        "category": event.category_name,
                        "title": event.title,
                        "is_live": existing_live,
                        "updated": updated,
                        "type": "channel_updated"
                    })
                }
                Ok(None) => {
                    json!({
                        "broadcaster_id": event.broadcaster_user_id.clone(),
                        "category": event.category_name,
                        "title": event.title,
                        "is_live": false,
                        "updated": Utc::now().to_rfc3339(),
                        "type": "channel_updated"
                    })
                }
                Err(e) => {
                    eprintln!("Failed to read existing streamer before mutation: {}", e);
                    return Ok(Response::builder().status(500).body(Body::Empty).unwrap());
                }
            }
        }

        _ => {
            return Ok(Response::builder()
                .status(204)
                .body(Body::Text("OK".into()))
                .unwrap());
        }
    };

    let mutation = r#"
    mutation UpdateStreamer($broadcaster_id: String!, $category: String!, $title: String!, $is_live: Boolean!, $updated: AWSDateTime!, $type: String!) {
    updateStreamer(
        broadcaster_id: $broadcaster_id,
        category: $category,
        title: $title,
        is_live: $is_live,
        updated: $updated,
        type: $type
    ) {
            broadcaster_id
            broadcaster_name
            category
            title
            is_live
            updated
            type
        }
    }
    "#;

    if let Err(e) = post_graphql_to_appsync(&appsync_api_host, &region, mutation, variables).await {
        eprintln!("Failed posting to AppSync: {}", e);
        // Failed, maybe handle?
    }

    Ok(Response::builder()
        .status(204)
        .body(Body::Text("OK".into()))
        .unwrap())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(function_handler)).await
}
