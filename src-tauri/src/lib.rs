pub mod command;
mod oauth;
mod util;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use keyring::{Entry, Error as EntryError};
use reqwest::blocking::Client as BlockingClient;
use rouille::{router, Response, Server};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::command::{login, on_startup, shutdown_server, ServerCtl};
use crate::oauth::{refresh_access_token, validate_access_token, verify_id_token};
use crate::util::load_secret;

fn set_window_size(window: &WebviewWindow) {
    let opt_montior = window.current_monitor().unwrap();
    let monitor = match opt_montior {
        Some(m) => m,
        None => {
            panic!("No monitor detected.")
        }
    };

    window
        .set_size(PhysicalSize {
            width: 500.0,
            height: monitor.size().height as f64,
        })
        .unwrap();
}

fn set_window_position(window: &WebviewWindow) {
    let opt_monitor = window.current_monitor().unwrap();

    let monitor = match opt_monitor {
        Some(m) => m,
        None => {
            panic!("Wtf no monitor?")
        }
    };

    let size = window.inner_size().unwrap();

    window
        .set_position(PhysicalPosition {
            x: (monitor.size().width - size.width) as f64,
            y: 0.0,
        })
        .unwrap();
}

fn handle_setup_user(
    app: AppHandle,
    csrf_state: String,
    nonce: Arc<Mutex<Option<String>>>,
    code_verifier: Arc<Mutex<Option<String>>>,
) -> ServerCtl {
    let server = Server::new("127.0.0.1:1337", move |request| {
        router!(request,
            (GET) (/) => {
                let qs = request.raw_query_string();
                let params: HashMap<_,_> = url::form_urlencoded::parse(qs.as_bytes()).into_owned().collect();

                let code = params.get("code").ok_or_else(|| Response::text("Missing code").with_status_code(400)).unwrap().to_string();
                let returned_state = params.get("state").ok_or_else(|| Response::text("Missing state").with_status_code(400)).unwrap().to_string();

                if returned_state != csrf_state {
                    return Response::text("Invalid state").with_status_code(400);
                }

                let nonce_value = {
                    let mut guard = nonce.lock().unwrap();
                    guard.take().ok_or_else(|| Response::text("Nonce already used").with_status_code(400)).unwrap()
                };

                if let Some(returned_challenge) = params.get("code_challenge") {
                    let expected = {
                        let guard = code_verifier.lock().unwrap();
                        let maybe_v = guard.as_ref().expect("verifier already consumed");
                        let digest = Sha256::digest(maybe_v.as_bytes());
                        URL_SAFE_NO_PAD.encode(digest)
                    };
                    if returned_challenge != &expected {
                        return Response::text("PKCE challenge mismatch").with_status_code(400);
                    }
                }

                let verifier = {
                    let mut lock = code_verifier.lock().unwrap();
                    lock.take().expect("PKCE verifier already consumed")
                };

                let client_id = std::env::var("CLIENT_ID").expect("CLIENT_ID env not set");
                let client_secret = std::env::var("CLIENT_SECRET").expect("CLIENT_SECRET env not set");
                let redirect_uri = std::env::var("REDIRECT_URI").expect("REDIRECT_URI env not set");

                let http_client = BlockingClient::new();
                let params = [
                    ("client_id", client_id.as_str()),
                    ("client_secret", client_secret.as_str()),
                    ("grant_type", "authorization_code"),
                    ("code", code.as_str()),
                    ("redirect_uri", redirect_uri.as_str()),
                    ("code_verifier", verifier.as_str()),
                ];

                let resp = match http_client.post("https://id.twitch.tv/oauth2/token").form(&params).send() {
                    Ok(r) => r,
                    Err(e) => return Response::text(format!("Network error: {:?}", e)).with_status_code(500)
                };
                let status = resp.status();
                let body = match resp.text() {
                    Ok(t) => t,
                    Err(e) => return Response::text(format!("Failed to read token body: {:?}", e)).with_status_code(500)
                };

                if status.is_success() {
                    let token_val: Value = match serde_json::from_str(&body) {
                        Ok(v) => v,
                        Err(e) => return Response::text(format!("Invalid token JSON: {:?}", e)).with_status_code(500)
                    };

                    let id_token_str = match token_val.get("id_token").and_then(|v| v.as_str()) {
                        Some(s) => s,
                        None => return Response::text("no id_token in response").with_status_code(500)
                    };

                    let jwks_uri = "https://id.twitch.tv/oauth2/keys";
                    let expected_issuer = "https://id.twitch.tv/oauth2";

                    let claims = match verify_id_token(id_token_str, jwks_uri, expected_issuer, &client_id, Some(&nonce_value)) {
                        Ok(c) => c,
                        Err(e) => return Response::text(format!("id_token verify failed: {:?}", e)).with_status_code(400)
                    };

                    Entry::new("notisr", "id_token").expect("failed to create entry").set_secret(id_token_str.as_bytes()).expect("failed to set id_token to entry");

                    if let Some(access_token) = token_val.get("access_token").and_then(|v| v.as_str()) {
                        Entry::new("notisr", "access_token").unwrap()
                            .set_secret(access_token.as_bytes()).unwrap();
                    }

                    Entry::new("notisr", "user_id").expect("Failed to create user_id entry").set_secret(claims.sub.as_bytes()).expect("failed to set user_id to entry");

                    if let Some(refresh_token) = token_val.get("refresh_token").and_then(|v| v.as_str()) {
                        Entry::new("notisr", "refresh_token").expect("failed to create refresh_token entry").set_secret(refresh_token.as_bytes()).expect("failed to set refresh_token");
                    }

                    if let Some(win) = app.get_webview_window("login") { let _ = win.close(); }
                    if let Some(window) = app.get_webview_window("main") {
                        window.try_state::<Mutex<Option<String>>>().unwrap().lock().unwrap().take();
                        let _ = window.emit("logged_in", ());
                        set_window_size(&window);
                        set_window_position(&window);
                    }

                    Response::text("Login successful—token stored securely!")
                } else if status.is_client_error() {
                    Response::text(format!("Client error: {}, body: {}", status, body)).with_status_code(400)
                } else if status.is_server_error() {
                    Response::text(format!("Server error: {}, body: {}", status, body)).with_status_code(500)
                } else {
                    Response::text(format!("Unexpected status: {}, body: {}", status, body)).with_status_code(500)
                }
            },
            _ => Response::empty_404()
        )
    }).expect("Failed to start server");

    let (handle, stop_tx) = server.stoppable();
    ServerCtl { stop_tx, handle }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let auth_state: Mutex<Option<String>> = Mutex::new(None);
            app.manage(auth_state);

            // Switch out to utils.load_secret
            let entry = Entry::new("notisr", "access_token").unwrap();

            let decision = match entry.get_secret() {
                Ok(access_token) => {
                    let access_token_str = str::from_utf8(&access_token).unwrap();
                    match validate_access_token(&access_token_str) {
                        Ok(Some(resp)) => {
                            if resp.expires_in.unwrap_or(0) > 0 {
                                Some(());
                            }
                            eprintln!("Access token appears expired, attempting refresh...");
                        }
                        Ok(None) => {
                            eprintln!("Access token invalid (401). Will try refresh if possible.");
                        }
                        Err(e) => {
                            eprintln!("validate error during setup: {:?}", e);
                            Some("log_in".to_string());
                        }
                    }

                    let refresh_token = match load_secret("refresh_token") {
                        Some(r) => r,
                        None => "log_in".to_string(),
                    };
                    match refresh_access_token(&refresh_token) {
                        Ok(()) => None,
                        Err(e) => {
                            eprint!("Refresh failed: {:?}", e);
                            Some("log_in".to_string())
                        }
                    }
                }
                Err(e) => match e {
                    EntryError::NoEntry => Some("log_in".to_string()),
                    _ => {
                        panic!("Something went horribly wrong: {:?}", e)
                    }
                },
            };
            *app.state::<Mutex<Option<String>>>().lock().unwrap() = decision;
            Ok(())
        })
        .on_page_load(|webview, _payload| {
            let x = webview.app_handle();
            let v = x
                .try_state::<Mutex<Option<String>>>()
                .unwrap()
                .lock()
                .unwrap()
                .clone();

            if v.is_none() {
                set_window_position(&webview.window().get_webview_window("main").unwrap());
                set_window_size(&webview.window().get_webview_window("main").unwrap());
            }
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_positioner::init())
        .invoke_handler(tauri::generate_handler![login, shutdown_server, on_startup])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
