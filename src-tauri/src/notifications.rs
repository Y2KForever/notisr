use tauri::AppHandle;

pub fn send_notification(
  title: String,
  content: String,
  name: String,
  app_handle: AppHandle,
) {
  #[cfg(target_os = "macos")]
  {
    use mac_notification_sys::{MainButton, Notification};
    use tauri_plugin_opener::OpenerExt;

    let notification_res: Result<
      mac_notification_sys::NotificationResponse,
      mac_notification_sys::error::Error,
    > = Notification::new()
      .main_button(MainButton::SingleAction("Open stream"))
      .title(&title)
      .message(&content)
      .default_sound()
      .send();

    match notification_res {
      Ok(notification_resp) => match notification_resp {
        mac_notification_sys::NotificationResponse::ActionButton(_) => {
          let _ = app_handle
            .opener()
            .open_url(format!("https://twitch.tv/{}", name), None::<&str>)
            .unwrap();
        }
        mac_notification_sys::NotificationResponse::Click => {
          let _ = app_handle
            .opener()
            .open_url(format!("https://twitch.tv/{}", name), None::<&str>)
            .unwrap();
        }
        mac_notification_sys::NotificationResponse::CloseButton(_) => {}
        mac_notification_sys::NotificationResponse::None => {}
        mac_notification_sys::NotificationResponse::Reply(_) => {}
      },
      Err(e) => {
        println!("Error creating notification: {:?}", e)
      }
    }
  }

  #[cfg(target_os = "windows")]
  {
    use std::str::FromStr;
    use tauri_plugin_notification::{Attachment, NotificationExt};
    use url::Url;

    let link = format!("https://twitch.tv/{}", name);
    let url = Url::from_str(&link).unwrap();
    let id = uuid::Uuid::new_v4().to_string();

    let _ = app_handle
      .notification()
      .builder()
      .title(title)
      .body(content)
      .show()
      .expect("Failed to deliver notifications");
  }
}
