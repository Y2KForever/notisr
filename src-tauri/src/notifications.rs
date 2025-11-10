#[cfg(target_os = "macos")]
use std::thread;
use tauri::AppHandle;

pub fn send_notification(
  title: String,
  content: String,
  name: String,
  #[allow(unused)] app_handle: AppHandle,
) {
  #[cfg(target_os = "macos")]
  {
    use mac_notification_sys::{MainButton, Notification};
    use tauri_plugin_opener::OpenerExt;

    let title_clone = title.clone();
    let content_clone = content.clone();
    let name_clone = name.clone();
    let app_handle_clone = app_handle.clone();

    thread::spawn(move || {
      let notification_res = Notification::new()
        .main_button(MainButton::SingleAction("Open stream"))
        .title(&title_clone)
        .message(&content_clone)
        .default_sound()
        .send();

      match notification_res {
        Ok(notification_resp) => match notification_resp {
          mac_notification_sys::NotificationResponse::ActionButton(_) => {
            let _ = app_handle_clone.opener().open_url(
              format!("https://twitch.tv/{}", name_clone),
              None::<&str>,
            );
          }
          mac_notification_sys::NotificationResponse::Click => {
            let _ = app_handle_clone.opener().open_url(
              format!("https://twitch.tv/{}", name_clone),
              None::<&str>,
            );
          }
          mac_notification_sys::NotificationResponse::CloseButton(_) => {}
          mac_notification_sys::NotificationResponse::None => {}
          mac_notification_sys::NotificationResponse::Reply(_) => {}
        },
        Err(e) => {
          use tauri_plugin_notification::NotificationExt;

          println!("Error creating notification: {:?}", e);
          // Fallback to default Tauri notification if we fail to deliver.
          app_handle_clone
            .notification()
            .builder()
            .title(&title_clone)
            .body(&content_clone)
            .show()
            .expect("Failed to deliver tauri notification");
        }
      }
    });
  }

  #[cfg(target_os = "windows")]
  {
    use universal_notifications::Windows::{
      ActivationType, Duration, Sound, Toast,
    };

    let url = format!("https://twitch.tv/{}", name);

    Toast::new("com.y2kforever.notisr")
      .title(&title)
      .description(&content)
      .duration(Duration::Long)
      .sound(Some(Sound::Default))
      .action("Open Stream", &url, ActivationType::Protocol)
      .show()
      .expect("Failed to deliver notifications");
  }
}
