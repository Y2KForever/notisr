mod appsync;
pub mod command;
mod oauth;
mod twitch;
mod util;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use dotenvy_macro::dotenv;
use keyring::Entry;
use reqwest::blocking::Client as BlockingClient;
use rouille::{router, Response, Server};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{
  MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
};
use tauri::{
  AppHandle, Emitter, LogicalPosition, Manager, PhysicalSize, RunEvent,
  WebviewUrl, WebviewWindow, WindowEvent,
};
use tauri_plugin_notification::{NotificationExt, PermissionState};

use crate::appsync::{start_ws_client, stop_ws_client};
use crate::command::{
  fetch_streamers, login, on_startup, open_broadcaster_url, shutdown_server,
  ServerCtl,
};
use crate::util::{check_validitiy_token, spawn_new_user};
#[derive(Serialize, Deserialize, Debug)]
struct UserInfo {
  user_id: String,
}

fn set_window_size(window: &WebviewWindow) {
  let opt_monitor = window.current_monitor().unwrap();

  let monitor = match opt_monitor {
    Some(m) => m,
    None => {
      panic!("Wtf no monitor?")
    }
  };

  window
    .set_size(PhysicalSize {
      width: 250.0,
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

  let x = ((monitor.size().width as f64) / monitor.scale_factor()) - 250.0; // Remove magic number
  let y = 0.0;

  window.set_position(LogicalPosition { x: x, y: y }).unwrap();
}

fn handle_setup_user(
  app: AppHandle,
  csrf_state: String,
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

                let client_id = dotenv!("CLIENT_ID");
                let client_secret = dotenv!("CLIENT_SECRET");
                let redirect_uri = dotenv!("REDIRECT_URI");

                let http_client = BlockingClient::new();
                let params = [
                    ("client_id", client_id),
                    ("client_secret", client_secret),
                    ("grant_type", "authorization_code"),
                    ("code", code.as_str()),
                    ("redirect_uri", redirect_uri),
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

                    let access_token = match token_val.get("access_token").and_then(|v| v.as_str()) {
                      Some(token) => {token.to_owned()}
                      None => panic!("Access token did not exist in json")
                    };

                    let client = BlockingClient::new();
                    let validation_response = match client.get("https://id.twitch.tv/oauth2/validate").header("Authorization", format!("Bearer {}", access_token)).send() {
                      Ok(r) => {r},
                      Err(e) => {return Response::text(format!("Network error: {:?}", e)).with_status_code(500)},
                    };

                    let user_info: UserInfo = match validation_response.json() {
                        Ok(v) => v,
                        Err(e) => { return Response::text(format!("Failed to parse JSON: {:?}", e)).with_status_code(500) },
                    };

                    let access_token_cloned = access_token.clone();
                    let user_id = user_info.user_id;
                    let app_cloned = app.clone();
                    let access_token_ws = access_token.clone();

                    Entry::new("notisr", "access_token").unwrap()
                        .set_secret(access_token.as_bytes()).unwrap();

                    Entry::new("notisr", "user_id").expect("Failed to create user_id entry")
                    .set_secret(user_id.as_bytes()).expect("failed to set user_id to entry");

                    spawn_new_user(access_token_cloned, user_id, access_token_ws, app_cloned);

                    if let Some(refresh_token) = token_val.get("refresh_token").and_then(|v| v.as_str()) {
                        Entry::new("notisr", "refresh_token").expect("failed to create refresh_token entry").set_secret(refresh_token.as_bytes()).expect("failed to set refresh_token");
                    }

                    if let Some(win) = app.get_webview_window("login") { let _ = win.close(); }
                    if let Some(window) = app.get_webview_window("main") {
                        window.try_state::<Mutex<Option<String>>>().unwrap().lock().unwrap().take();
                        let _ = window.emit("logged_in", ());
                        set_window_size(&window);
                        set_window_position(&window);

                        let _ = window.show();
                        let _ = window.set_focus();
                    }

                    Response::text("Login successful!\n\nYou can now close this window.")
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
  let builder = tauri::Builder::default()
    .plugin(tauri_plugin_notification::init())
    .setup(|app| {
      let show_menu_on_left_click = cfg!(target_os = "macos");

      let quit_item =
        MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
      let show_item =
        MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;

      let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
      let decision = check_validitiy_token();
      let needs_login = decision.is_none();
      let main_window = tauri::WebviewWindowBuilder::new(
        app,
        "main",
        WebviewUrl::App("index.html".into()),
      )
      .title("Notisr")
      .inner_size(500.0, 800.0)
      .min_inner_size(250.0, 100.0)
      .visible(false)
      .build()
      .unwrap();

      let _ = TrayIconBuilder::new()
        .on_menu_event(|app, event| match event.id.as_ref() {
          "show" => {
            if let Some(window) = app.get_webview_window("main") {
              let _ = window.show();
              let _ = window.set_focus();
            }
          }
          "quit" => app.exit(0),
          _ => {}
        })
        .menu(&menu)
        .show_menu_on_left_click(show_menu_on_left_click)
        .icon(app.default_window_icon().unwrap().clone())
        .on_tray_icon_event(|tray, event| match event {
          TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
          } => {
            #[cfg(target_os = "macos")]
            {
              let app = tray.app_handle();
              if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
              }
            }
          }
          _ => {}
        })
        .build(app)?;

      let auth_state: Mutex<Option<String>> = Mutex::new(None);
      app.manage(auth_state);

      *app.state::<Mutex<Option<String>>>().lock().unwrap() = decision.clone();

      if needs_login {
        if let Some(window) = app.get_webview_window("main") {
          let _ = window.show();
        }
      } else {
        set_window_size(&main_window);
        set_window_position(&main_window);
        if let Some(token) = &decision {
          let token = token.clone();
          let app_handle = app.handle().clone();
          std::thread::spawn(move || {
            if let Err(e) = start_ws_client(app_handle, token) {
              eprintln!("WebSocket client failed to start: {:?}", e);
            }
          });
        }
      }

      match app.notification().permission_state() {
        Ok(permission_state) => {
          if permission_state != PermissionState::Granted {
            let _ = app.notification().request_permission().unwrap();
          }
        }
        Err(e) => {
          println!("Error while trying to get permission state: {:?}", e)
        }
      }

      Ok(())
    })
    .plugin(tauri_plugin_opener::init())
    .plugin(tauri_plugin_positioner::init())
    .plugin(tauri_plugin_notification::init())
    // .plugin(tauri_plugin_updater::Builder::new().build())
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_process::init())
    .invoke_handler(tauri::generate_handler![
      shutdown_server,
      on_startup,
      login,
      open_broadcaster_url,
      fetch_streamers
    ]);

  let context = tauri::generate_context!();
  #[allow(unused_mut)]
  let mut app = builder.build(context).expect("Error while building app");

  app.run(move |app_handle, event| match event {
    RunEvent::WindowEvent { label, event, .. } => {
      if label == "main" {
        if let WindowEvent::CloseRequested { api, .. } = event {
          api.prevent_close();
          if let Some(win) = app_handle.get_webview_window("main") {
            let _ = win.hide();
          }
        }
      }
    }
    RunEvent::ExitRequested { .. } => {
      if let Err(e) = stop_ws_client() {
        eprintln!("Failed to stop the ws client. Error: {:?}", e);
      }
    }
    _ => {}
  });
}
