use std::{
    sync::mpsc::Sender,
    sync::{Arc, Mutex},
    thread::JoinHandle,
};

use crate::{
    handle_setup_user,
    oauth::{gen_b64_url, generate_pkce_pair},
};
use tauri::{AppHandle, Manager};
use url::Url;

pub struct ServerCtl {
    pub stop_tx: Sender<()>,
    pub handle: JoinHandle<()>,
}

#[tauri::command]
pub fn shutdown_server(state: tauri::State<std::sync::Mutex<Option<ServerCtl>>>) {
    let mut guard = state.inner().lock().unwrap();
    if let Some(ctl) = guard.take() {
        let _ = ctl.stop_tx.send(());
        std::thread::spawn(move || {
            let _ = ctl.handle.join();
        });
    }
}

#[tauri::command]
pub fn on_startup(state: tauri::State<'_, Mutex<Option<String>>>) -> Option<String> {
    state.lock().unwrap().clone()
}

#[tauri::command]
pub fn login(app: AppHandle) {
    dotenvy::dotenv().ok();
    let client_id = std::env::var("CLIENT_ID").expect("CLIENT_ID env not set");
    let redirect_uri = std::env::var("REDIRECT_URI").expect("REDIRECT_URI env not set");
    let scope = std::env::var("SCOPE").expect("SCOPE env not set");

    let (pkce_challenge, pkce_verifier) = generate_pkce_pair();
    let csrf_state = gen_b64_url();
    let nonce = gen_b64_url();

    let verifier_arc = Arc::new(Mutex::new(Some(pkce_verifier)));
    let nonce_arc = Arc::new(Mutex::new(Some(nonce.clone())));

    let mut auth_url = Url::parse("https://id.twitch.tv/oauth2/authorize").expect("valid base url");

    auth_url
        .query_pairs_mut()
        .append_pair("force_verify", "true")
        .append_pair("response_type", "code")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", &scope)
        .append_pair("state", &csrf_state)
        .append_pair("code_challenge", &pkce_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("nonce", &nonce);

    let url_string = auth_url.clone();

    let ctl = handle_setup_user(app.clone(), csrf_state, nonce_arc, verifier_arc.clone());
    app.manage(std::sync::Mutex::new(Some(ctl)));

    let _ =
        tauri::WebviewWindowBuilder::new(&app, "login", tauri::WebviewUrl::External(url_string))
            .title("Login with Twitch")
            .inner_size(800.0, 600.0)
            .build()
            .unwrap();
}
