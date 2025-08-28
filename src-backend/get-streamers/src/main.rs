use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::types::{AttributeValue, KeysAndAttributes};
use aws_sdk_dynamodb::Client as DynamoDbClient;
use futures::future::join_all;
use lambda_http::{run, service_fn, Body, Error, Request, Response};
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;

const STREAMER_TABLE_ENV: &str = "STREAMER_TABLE";
const CHUNK_SIZE: usize = 100;
const MAX_RETRIES: usize = 5;
const BACKOFF_BASE_MS: u64 = 50u64;

fn attr_to_json(av: &AttributeValue) -> serde_json::Value {
    if let Some(s) = av.as_s().ok() {
        serde_json::Value::String(s.to_string())
    } else if let Some(n) = av.as_n().ok() {
        serde_json::Value::String(n.to_string())
    } else if let Some(b) = av.as_bool().ok() {
        serde_json::Value::Bool(*b)
    } else {
        serde_json::Value::Null
    }
}

async fn batch_get_chunk(
    ddb: DynamoDbClient,
    table_name: String,
    keys: Vec<HashMap<String, AttributeValue>>,
) -> Result<Vec<HashMap<String, AttributeValue>>, String> {
    let kaa = KeysAndAttributes::builder()
        .set_keys(Some(keys.clone()))
        .build()
        .map_err(|e| format!("KeysAndAttributes build error: {}", e))?;

    let mut request_items: HashMap<String, KeysAndAttributes> = HashMap::new();
    request_items.insert(table_name.clone(), kaa);

    let mut attempt = 0usize;

    loop {
        attempt += 1;
        let resp = ddb
            .batch_get_item()
            .set_request_items(Some(request_items.clone()))
            .send()
            .await;

        match resp {
            Ok(output) => {
                let mut returned: Vec<HashMap<String, AttributeValue>> = Vec::new();
                if let Some(map) = output.responses() {
                    if let Some(items) = map.get(&table_name) {
                        returned.extend(items.clone());
                    }
                }

                if let Some(unprocessed) = output.unprocessed_keys() {
                    if let Some(uka) = unprocessed.get(&table_name) {
                        if !uka.keys().is_empty() && attempt <= MAX_RETRIES {
                            let keys = uka.keys().to_owned();
                            for key in keys {
                                let retry_kaa =
                                    KeysAndAttributes::builder().keys(key).build().map_err(
                                        |e| format!("retry KeysAndAttributes build error: {}", e),
                                    )?;

                                let mut retry_map = HashMap::new();
                                retry_map.insert(table_name.clone(), retry_kaa);
                                request_items = retry_map;
                            }

                            let sleep_ms = BACKOFF_BASE_MS.saturating_mul(2u64.pow(attempt as u32));
                            tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                            continue;
                        }
                    }
                }

                return Ok(returned);
            }
            Err(e) => {
                if attempt >= MAX_RETRIES {
                    return Err(format!("batch_get_item failed after retries: {}", e));
                } else {
                    let sleep_ms = BACKOFF_BASE_MS.saturating_mul(2u64.pow(attempt as u32));
                    tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                    continue;
                }
            }
        }
    }
}

async fn function_handler(request: Request) -> Result<Response<Body>, Error> {
    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let ddb_client = DynamoDbClient::new(&config);
    let table_name = std::env::var(STREAMER_TABLE_ENV).expect("STREAMER_TABLE env required");

    let body_str = match request.body() {
        Body::Text(s) => s.clone(),
        _ => String::new(),
    };

    let broadcaster_ids: Vec<String> = match serde_json::from_str(&body_str) {
        Ok(v) => v,
        Err(e) => {
            let err = json!({ "error": format!("Invalid request JSON: {}", e) });
            let body = Body::Text(err.to_string());
            return Ok(Response::builder().status(400).body(body).unwrap());
        }
    };

    if broadcaster_ids.is_empty() {
        let body = Body::Text("[]".to_string());
        return Ok(Response::builder().status(200).body(body).unwrap());
    }

    let chunks: Vec<Vec<String>> = broadcaster_ids
        .chunks(CHUNK_SIZE)
        .map(|c| c.to_vec())
        .collect();

    let mut futures = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let ddb = ddb_client.clone();
        let table_name = table_name.clone();
        let keys: Vec<HashMap<String, AttributeValue>> = chunk
            .iter()
            .map(|id| {
                let mut key = HashMap::new();
                key.insert("broadcaster_id".to_string(), AttributeValue::S(id.clone()));
                key
            })
            .collect();

        futures.push(batch_get_chunk(ddb, table_name, keys));
    }

    let chunk_results = join_all(futures).await;

    for cr in &chunk_results {
        if let Err(e) = cr {
            eprintln!("chunk error: {}", e);
        }
    }

    let mut id_to_item: HashMap<String, HashMap<String, AttributeValue>> = HashMap::new();
    for chunk_vec in chunk_results.into_iter() {
        if let Ok(items) = chunk_vec {
            for item in items {
                if let Some(av) = item.get("broadcaster_id") {
                    let bid = av.as_s();
                    match bid {
                        Ok(id) => {
                            id_to_item.insert(id.clone(), item);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let mut out: Vec<Option<serde_json::Map<String, serde_json::Value>>> =
        Vec::with_capacity(broadcaster_ids.len());
    for id in broadcaster_ids {
        if let Some(item) = id_to_item.get(&id) {
            let mut json_map = serde_json::Map::new();
            for (k, v) in item {
                json_map.insert(k.clone(), attr_to_json(v));
            }
            out.push(Some(json_map));
        } else {
            out.push(None);
        }
    }

    let resp_text = serde_json::to_string(&out).unwrap_or_else(|_| "[]".to_string());
    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(Body::Text(resp_text))
        .unwrap())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(function_handler)).await
}
