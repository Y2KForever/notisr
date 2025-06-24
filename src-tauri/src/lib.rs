use dirs;
use rand::{distr::Alphanumeric, Rng};
use rouille::{router, Response};
use std::{
    env, fs,
    io::{self, ErrorKind},
    path::PathBuf,
    sync::mpsc::Sender,
    thread,
};
use tauri::{AppHandle, Emitter, Manager, Url};

#[derive(Clone)]
struct ServerCtl {
    stop_tx: Sender<()>,
}

#[tauri::command]
fn shutdown_server(state: tauri::State<Option<ServerCtl>>) {
    if let Some(ctl) = state.inner().as_ref() {
        let _ = ctl.stop_tx.send(());
    }
}

#[tauri::command]
fn login(app: AppHandle) {
    dotenvy::dotenv().ok();
    let client_id = env::var("CLIENT_ID").expect("CLIENT_ID env not set");
    let redirect_uri = env::var("REDIRECT_URI").expect("REDIRECT_URI env not set");
    let scope = env::var("SCOPE").expect("SCOPE env not set");
    let state: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(7)
        .map(char::from)
        .collect();
    let uri = Url::parse(format!("https://id.twitch.tv/oauth2/authorize?force_verify=true&response_type=code&client_id={}&redirect_uri={}&scope={}&state={}", client_id, redirect_uri, scope, state).as_str()).expect("Failed to parse URL");
    let _new_win =
        tauri::WebviewWindowBuilder::new(&app, "login", tauri::WebviewUrl::External(uri))
            .title("Login")
            .inner_size(800.0, 600.0)
            .build()
            .unwrap();
}

fn handle_setup_user(path: PathBuf, app: AppHandle) -> ServerCtl {
    let server = rouille::Server::new("localhost:1337", move | request | {
        router!(request,
            (GET) (/) => {
                let code = match request.get_param("code") {
                    Some(c) => c,
                    None => {
                        return Response::text("Missing `code` query parameter").with_status_code(400);
                    }
                };
                let contents = format!(r#"{{"code":"{}"}}"#, code);
                let parent = &path.parent().unwrap();
                if let Err(e) = fs::create_dir_all(parent){
                    eprintln!("Failed to create config directory {:?}", e);
                }
                if let Err(err) = fs::write(&path, &contents){
                    eprintln!("Failed to write file. Error: {:?}", err);
                }

                app.get_webview_window("login").unwrap().close().unwrap();
                let window = app.get_webview_window("main").unwrap();
                window.emit("logged_in", ()).unwrap();

                Response::empty_204()
            },
            _ => Response::empty_404()
        )
    }).unwrap();
    let (_, stop_tx) = server.stoppable();
    ServerCtl { stop_tx }
}

async fn first_time_run(file_name: String, app: AppHandle) -> io::Result<()> {
    let mut data_dir = dirs::data_local_dir()
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "couldn't determine local data directory",
            )
        })
        .unwrap();
    data_dir.push("notisr");
    data_dir.push(file_name);
    match tokio::fs::read_to_string(&data_dir).await {
        Ok(_contents) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            println!("File not found. Starting web server.");
            thread::spawn(|| handle_setup_user(data_dir, app));
            Ok(())
        }
        Err(e) => {
            println!("Failed to read config.json: {:?}", e);
            Err(e)
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();
            tauri::async_runtime::block_on(first_time_run("config.json".into(), app_handle))?;
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![login, shutdown_server])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
