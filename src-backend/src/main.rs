use aws_config::meta::region::RegionProviderChain;
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::{Client as DynamoDbClient, Error as DynamoError};
use aws_sdk_secretsmanager::Client as SecretsClient;
use chrono::Utc;
use hmac::{Hmac, Mac};
use lambda_http::http::HeaderMap;
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use serde::Deserialize;
use serde_json::Value;
use sha2::Sha256;

const STREAMER_TABLE_ENV: &str = "STREAMER_TABLE";
const SECRET_ARN_ENV: &str = "SECRET_ARN";

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
    broadcaster_user_name: String,
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

    // Compute HMAC-SHA256 over that message
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC init failed");
    mac.update(message.as_bytes());
    let computed = mac.finalize().into_bytes();
    let computed_hex = hex::encode(computed);
    let computed_sig = format!("sha256={}", computed_hex);
    println!("DEBUG: computed signature = {}", computed_sig);

    // Constant-time compare
    let valid = sig_header.eq_ignore_ascii_case(&computed_sig);
    println!("DEBUG: signature valid = {}", valid);
    valid
}

async fn update_streamer(
    payload: &TwitchWebhookEvent,
    ddb_client: &DynamoDbClient,
    table_name: &str,
) -> Result<(), DynamoError> {
    let timestamp = Utc::now().to_rfc3339();

    match payload.subscription.event_type.as_str() {
        "stream.online" | "stream.offline" => {
            let event: StreamEvent = serde_json::from_value(payload.event.clone()).unwrap();
            let is_live = payload.subscription.event_type == "stream.online";

            ddb_client
                .update_item()
                .table_name(table_name)
                .key(
                    "broadcaster_id",
                    AttributeValue::S(event.broadcaster_user_id),
                )
                .update_expression("SET isLive = :live, lastUpdated = :ts")
                .expression_attribute_values(":live", AttributeValue::Bool(is_live))
                .expression_attribute_values(":ts", AttributeValue::S(timestamp.clone()))
                .send()
                .await?;
        }
        "channel.update" => {
            let event: ChannelUpdateEvent = serde_json::from_value(payload.event.clone()).unwrap();

            ddb_client
                .update_item()
                .table_name(table_name)
                .key("broadcaster_id", AttributeValue::S(event.broadcaster_user_id.clone()))
                .update_expression(
                    "SET displayName = :name, title = :title, category = :category, lastUpdated = :ts",
                )
                .expression_attribute_values(
                    ":name",
                    AttributeValue::S(event.broadcaster_user_name),
                )
                .expression_attribute_values(
                    ":title",
                    AttributeValue::S(event.title),
                )
                .expression_attribute_values(
                    ":category",
                    AttributeValue::S(event.category_name),
                )
                .expression_attribute_values(
                    ":ts",
                    AttributeValue::S(timestamp.clone()),
                )
                .send()
                .await?;
        }
        _ => {}
    }

    Ok(())
}

async fn function_handler(request: Request) -> Result<Response<Body>, Error> {
    // Initialize AWS clients
    let region_provider = RegionProviderChain::default_provider().or_else("eu-west-1");
    let config = aws_config::from_env().region(region_provider).load().await;
    let secrets_client = SecretsClient::new(&config);
    let ddb_client = DynamoDbClient::new(&config);

    // Get environment variables
    let table_name =
        std::env::var(STREAMER_TABLE_ENV).expect("STREAMER_TABLE environment variable not set");

    // Get secret
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

    // Handle challenge
    if let Ok(challenge) = serde_json::from_str::<ChallengePayload>(&body_str) {
        return Ok(Response::builder()
            .status(200)
            .header("Content-Type", "text/plain")
            .body(Body::Text(challenge.challenge))
            .unwrap());
    }

    // Parse full payload
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

    // Update DynamoDB
    if let Err(e) = update_streamer(&payload, &ddb_client, &table_name).await {
        eprintln!("DynamoDB update failed: {:?}", e);
        return Ok(Response::builder().status(500).body(Body::Empty).unwrap());
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
