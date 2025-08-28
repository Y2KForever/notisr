use std::collections::{HashMap, HashSet};

use aws_config::meta::region::RegionProviderChain;
use aws_sdk_dynamodb::types::{AttributeValue, KeysAndAttributes, PutRequest, WriteRequest};
use aws_sdk_dynamodb::Client as DynamoDbClient;
use aws_sdk_secretsmanager::Client as SecretsClient;
use chrono::Utc;
use futures::future::join_all;
use futures::FutureExt;
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use reqwest::Url;
use reqwest::{header::HeaderMap, header::CONTENT_TYPE, Client};
use serde::{Deserialize, Serialize};

const STREAMER_TABLE_ENV: &str = "STREAMER_TABLE";
const SECRET_ARN_ENV: &str = "SECRET_ARN";
const CALLBACK_URL_ENV: &str = "CALLBACK_URL";
const TOKEN_URL_ENV: &str = "TOKEN_URL";
const SUBSCRIPTION_URL_ENV: &str = "SUBSCRIPTION_URL";
const STREAMS_URL_ENV: &str = "STREAMS_URL";
const CHANNELS_URL_ENV: &str = "CHANNELS_URL";
const USERS_URL_ENV: &str = "USERS_URL";

#[derive(Deserialize, Debug)]
struct TwitchSecretConfig {
    webhook_secret: String,
    client_id: String,
    client_secret: String,
    grant_type: String,
}

#[derive(Deserialize, Debug)]
struct RegisterWebhookBody {
    broadcaster_id: u32,
}

#[derive(Deserialize, Debug)]
struct AuthResponse {
    access_token: String,
}

#[derive(Serialize)]
struct SubscriptionRequest<'a> {
    #[serde(rename = "type")]
    sub_type: &'a str,
    version: u8,
    condition: Condition<'a>,
    transport: Transport<'a>,
}

#[derive(Serialize)]
struct Condition<'a> {
    broadcaster_user_id: &'a str,
}

#[derive(Serialize)]
struct Transport<'a> {
    method: &'a str,
    callback: &'a str,
    secret: &'a str,
}

#[derive(Deserialize, Debug)]
struct StreamsResponse {
    data: Vec<StreamInfo>,
}

#[derive(Deserialize, Debug)]
struct StreamInfo {
    #[serde(rename = "type")]
    status: String,
    game_name: String,
    user_id: String,
    user_name: String,
    title: String,
}

#[derive(Deserialize, Debug, Clone)]
struct Streams {
    user_id: String,
    user_name: String,
    game_name: String,
    is_live: bool,
    title: String,
    profile_picture: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Channels {
    broadcaster_id: String,
    broadcaster_name: String,
    game_name: String,
    title: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct ChannelsResponse {
    data: Vec<Channels>,
}

#[derive(Deserialize)]
struct UsersResponse {
    data: Vec<Users>,
}

#[derive(Deserialize)]
struct Users {
    id: String,
    profile_image_url: String,
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

async fn ids_exist(
    broadcasters: &Vec<RegisterWebhookBody>,
    ddb_client: &DynamoDbClient,
    table_name: &str,
    secret: &TwitchSecretConfig,
) -> Result<HashSet<String>, Error> {
    let key_maps: Vec<HashMap<String, AttributeValue>> = broadcasters
        .iter()
        .map(|streamer: &RegisterWebhookBody| {
            let mut m = HashMap::new();
            m.insert(
                "broadcaster_id".to_string(),
                AttributeValue::S(streamer.broadcaster_id.to_string()),
            );
            m
        })
        .collect();
    let mut found_ids = HashSet::new();
    for chunk in key_maps.chunks(100) {
        let keys_and_attrs = KeysAndAttributes::builder()
            .set_keys(Some(chunk.to_vec()))
            .build();

        let resp = ddb_client
            .batch_get_item()
            .request_items(table_name, keys_and_attrs)
            .send()
            .await?;

        if let Some(res_map) = resp.responses {
            if let Some(items) = res_map.get(table_name) {
                for item in items {
                    if let Some(AttributeValue::S(n)) = item.get("broadcaster_id") {
                        if let Ok(parsed) = n.parse::<u32>() {
                            found_ids.insert(parsed);
                        }
                    }
                }
            }
        }
    }
    let missing_streamers: Vec<&RegisterWebhookBody> = broadcasters
        .iter()
        .filter(|b| !found_ids.contains(&b.broadcaster_id))
        .collect();

    let token_url =
        std::env::var(TOKEN_URL_ENV).expect("TOKEN_URL_ENV environment variable not set.");
    let client = reqwest::Client::builder().build()?;

    let params = [
        ("client_id", &secret.client_id),
        ("client_secret", &secret.client_secret),
        ("grant_type", &secret.grant_type),
    ];

    let auth_response = match client.post(&token_url).query(&params).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to send auth request: {:?}", e);
            return Err(e.into());
        }
    };

    let auth_resp: AuthResponse = auth_response
        .json()
        .await
        .expect("Failed to fetch access token");

    let mut headers = HeaderMap::new();

    headers.insert("Client-ID", secret.client_id.parse().unwrap());
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());

    let futures = missing_streamers
        .iter()
        .map(|broadcaster| {
            let client = client.clone();
            let headers = headers.clone();
            let token = auth_resp.access_token.clone();
            let id_str = broadcaster.broadcaster_id.to_string();
            let streams_url = std::env::var(STREAMS_URL_ENV).expect("STREAMS_URL_ENV not set");

            async move {
                let resp = client
                    .get(&streams_url)
                    .query(&[("user_id", &id_str)])
                    .bearer_auth(&token)
                    .headers(headers)
                    .send()
                    .await;

                if let Err(err) = &resp {
                    eprintln!("Error {:?}", err);
                    panic!("Error {:?}", err);
                }
                (id_str, resp)
            }
            .boxed()
        })
        .collect::<Vec<_>>();

    let res: Vec<(String, Result<reqwest::Response, reqwest::Error>)> = join_all(futures).await;
    let mut streams: Vec<Streams> = Vec::new();
    let token = auth_resp.access_token.clone();

    for (i, (broadcaster_id, result)) in res.into_iter().enumerate() {
        match result {
            Ok(resp) => {
                let stream: StreamsResponse = match resp.json().await {
                    Ok(x) => x,
                    Err(err) => {
                        eprintln!("Response {i}: failed to parse JSON: {:?}", err);
                        return Err(err.into());
                    }
                };

                if !stream.data.is_empty() {
                    for item in &stream.data {
                        streams.push(Streams {
                            game_name: item.game_name.to_string(),
                            user_id: item.user_id.to_string(),
                            user_name: item.user_name.to_string(),
                            is_live: if item.status == "live" { true } else { false },
                            title: item.title.to_string(),
                            profile_picture: None,
                        });
                    }
                } else {
                    let channels_url =
                        std::env::var(CHANNELS_URL_ENV).expect("CHANNELS_URL_ENV not set");

                    let channel_response = client
                        .get(channels_url)
                        .query(&[("broadcaster_id", &broadcaster_id)])
                        .bearer_auth(&token)
                        .headers(headers.clone())
                        .send()
                        .await
                        .map_err(|e| {
                            eprintln!("Failed to fetch channels: {:?}", e);
                            Error::from(e)
                        })?;

                    let channels: ChannelsResponse =
                        channel_response.json().await.map_err(|e| {
                            eprintln!("Error parsing channel json: {:?}", e);
                            Error::from(e)
                        })?;

                    println!("Channel response: {:?}", channels);

                    for ch in channels.data {
                        streams.push(Streams {
                            user_id: ch.broadcaster_id,
                            user_name: ch.broadcaster_name,
                            game_name: ch.game_name,
                            is_live: false,
                            title: ch.title,
                            profile_picture: None,
                        });
                    }
                }
            }
            Err(err) => {
                eprintln!("Failed response: {:?}", err);
                return Err(err.into());
            }
        }
    }

    let users_url = std::env::var(USERS_URL_ENV).expect("USERS_URL_ENV not set");

    let params: Vec<(&str, &str)> = streams.iter().map(|s| ("id", s.user_id.as_str())).collect();

    let url = Url::parse_with_params(&users_url, params)?;

    if streams.len() > 0 {
        let client = Client::new();
        let resp = client
            .get(url)
            .bearer_auth(&token)
            .headers(headers.clone())
            .send()
            .await?;

        println!("Resp: {:?}", resp);

        let user_resp = resp.json::<UsersResponse>().await?;
        let pictures: HashMap<_, _> = user_resp
            .data
            .into_iter()
            .map(|p| (p.id, p.profile_image_url))
            .collect();

        for s in streams.iter_mut() {
            s.profile_picture = pictures.get(&s.user_id).cloned()
        }
    }

    let mut newly_inserted = HashSet::new();
    for chunk in streams.chunks(25) {
        let write_requests: Vec<WriteRequest> = chunk
            .iter()
            .map(|streamer| {
                let mut item = HashMap::new();
                item.insert(
                    "broadcaster_id".to_string(),
                    AttributeValue::S(streamer.user_id.to_string()),
                );
                item.insert(
                    "broadcaster_name".to_string(),
                    AttributeValue::S(streamer.user_name.to_string()),
                );
                item.insert(
                    "category".to_string(),
                    AttributeValue::S(streamer.game_name.to_string()),
                );
                item.insert(
                    "title".to_string(),
                    AttributeValue::S(streamer.title.to_string()),
                );
                item.insert(
                    "updated".to_string(),
                    AttributeValue::S(Utc::now().to_rfc3339()),
                );
                item.insert(
                    "created".to_string(),
                    AttributeValue::S(Utc::now().to_rfc3339()),
                );
                item.insert(
                    "is_live".to_string(),
                    AttributeValue::Bool(streamer.is_live),
                );
                item.insert(
                    "profile_picture".to_string(),
                    AttributeValue::S(
                        streamer
                            .profile_picture
                            .clone()
                            .unwrap_or("".to_string())
                            .to_string(),
                    ),
                );
                let put_req = PutRequest::builder().set_item(Some(item)).build();
                WriteRequest::builder().put_request(put_req).build()
            })
            .collect();

        ddb_client
            .batch_write_item()
            .request_items(table_name.to_string(), write_requests)
            .send()
            .await?;

        newly_inserted.extend(chunk.iter().map(|s| s.user_id.clone()));
    }

    Ok(newly_inserted)
}

async fn register_webhook(
    broadcaster_ids: HashSet<String>,
    secret: &TwitchSecretConfig,
) -> Result<(), Error> {
    let token_url =
        std::env::var(TOKEN_URL_ENV).expect("TOKEN_URL_ENV environment variable not set.");
    let client = reqwest::Client::builder().build()?;
    let params = [
        ("client_id", &secret.client_id),
        ("client_secret", &secret.client_secret),
        ("grant_type", &secret.grant_type),
    ];

    let auth_response = match client.post(&token_url).query(&params).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("❌ Failed to send auth request: {:?}", e);
            return Err(e.into());
        }
    };

    let auth_resp: AuthResponse = auth_response
        .json()
        .await
        .expect("Failed to fetch access token");

    let mut headers = HeaderMap::new();

    headers.insert("Client-ID", secret.client_id.parse().unwrap());
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());

    let subscriptions = &[
        ("stream.online", 1u8),
        ("stream.offline", 1u8),
        ("channel.update", 2u8),
    ];

    let futures = broadcaster_ids
        .iter()
        .flat_map(|id| {
            let client = client.clone();
            let headers = headers.clone();
            let secret = secret.webhook_secret.clone();
            let token = auth_resp.access_token.clone();
            let id_str = id.to_string();
            let callback_url = std::env::var(CALLBACK_URL_ENV).expect("CALLBACK_URL_ENV not set");
            let subscription_url =
                std::env::var(SUBSCRIPTION_URL_ENV).expect("SUBSCRIPTION_URL_ENV not set");

            subscriptions.iter().map(move |(evt_type, ver)| {
                let client = client.clone();
                let headers = headers.clone();
                let secret = secret.clone();
                let token = token.clone();
                let id_str = id_str.clone();
                let callback_url = callback_url.clone();
                let subscription_url = subscription_url.clone();
                let evt_type = *evt_type;
                let ver = *ver;

                async move {
                    let req_body = SubscriptionRequest {
                        sub_type: evt_type,
                        version: ver,
                        condition: Condition {
                            broadcaster_user_id: &id_str,
                        },
                        transport: Transport {
                            method: "webhook",
                            callback: &callback_url,
                            secret: &secret,
                        },
                    };

                    let resp = client
                        .post(&subscription_url)
                        .bearer_auth(&token)
                        .headers(headers)
                        .json(&req_body)
                        .send()
                        .await;

                    if let Err(err) = &resp {
                        eprintln!("Failed {} for {}: {:?}", evt_type, id_str, err);
                    }
                    resp
                }
                .boxed()
            })
        })
        .collect::<Vec<_>>();

    let res = join_all(futures).await;

    for (i, result) in res.into_iter().enumerate() {
        match result {
            Ok(response) => {
                if !response.status().is_success() {
                    eprintln!("Request {i} failed with status: {}", response.status());
                }
            }
            Err(e) => {
                eprintln!("Request {i} failed to send {:?}", e);
            }
        }
    }

    Ok(())
}

async fn function_handler(request: Request) -> Result<Response<Body>, Error> {
    let streamer_table_name = std::env::var(STREAMER_TABLE_ENV)
        .expect("STREAMER_TABLE_ENV environment variable not set.");
    let region_provider = RegionProviderChain::default_provider().or_else("eu-west-1");
    let config = aws_config::from_env().region(region_provider).load().await;
    let secrets_client = SecretsClient::new(&config);
    let ddb_client = DynamoDbClient::new(&config);

    let secret = get_twitch_secret_config(&secrets_client)
        .await
        .expect("Failed to get Twitch secret");

    let body_str = match request.body() {
        Body::Text(s) => s.clone(),
        _ => String::new(),
    };

    let payload: Vec<RegisterWebhookBody> = match serde_json::from_str(&body_str) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Invalid JSON: {:?}", e);
            return Ok(Response::builder()
                .status(400)
                .body(Body::Text("Invalid JSON".into()))
                .unwrap());
        }
    };

    let newly_created_ids =
        match ids_exist(&payload, &ddb_client, &streamer_table_name, &secret).await {
            Ok(set) => set,
            Err(e) => {
                eprintln!("DynamoDB create ids failed: {:?}", e);
                return Ok(Response::builder().status(500).body(Body::Empty).unwrap());
            }
        };

    match register_webhook(newly_created_ids, &secret).await {
        Ok(_) => {}
        Err(e) => {
            eprintln!("Failed to register webhooks: {:?}", e);
            return Ok(Response::builder().status(500).body(Body::Empty).unwrap());
        }
    };

    Ok(Response::builder().status(200).body(Body::Empty).unwrap())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(function_handler)).await
}
