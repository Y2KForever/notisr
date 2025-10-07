use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::sync::{Mutex, OnceLock};

static DEV_SECRETS: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
const DEV_SECRETS_PATH: &str = "dev-secrets.json";

#[derive(Debug)]
pub struct DevEntry {
  service: String,
  username: String,
}

fn get_secrets() -> &'static Mutex<HashMap<String, Vec<u8>>> {
  DEV_SECRETS.get_or_init(|| {
    let data = match File::open(DEV_SECRETS_PATH) {
      Ok(file) => {
        serde_json::from_reader(BufReader::new(file)).unwrap_or_default()
      }
      Err(_) => HashMap::new(),
    };
    Mutex::new(data)
  })
}

impl DevEntry {
  pub fn new(service: &str, username: &str) -> Self {
    Self {
      service: service.to_string(),
      username: username.to_string(),
    }
  }

  pub fn get_secret(&self) -> Result<Vec<u8>, &'static str> {
    let key = format!("{}:{}", self.service, self.username);
    let guard = get_secrets().lock().unwrap();
    guard.get(&key).cloned().ok_or("Secret not found")
  }

  pub fn set_secret(&self, secret: &[u8]) -> Result<(), &'static str> {
    let key = format!("{}:{}", self.service, self.username);
    let mut guard = get_secrets().lock().unwrap();
    guard.insert(key, secret.to_vec());

    // Save to file
    let file =
      File::create(DEV_SECRETS_PATH).map_err(|_| "Failed to create file")?;
    serde_json::to_writer(BufWriter::new(file), &*guard)
      .map_err(|_| "Failed to write to file")?;

    Ok(())
  }

  pub fn delete_secret(&self) -> Result<(), &'static str> {
    let key = format!("{}:{}", self.service, self.username);
    let mut guard = get_secrets().lock().unwrap();
    guard.remove(&key);

    // Save to file
    let file =
      File::create(DEV_SECRETS_PATH).map_err(|_| "Failed to create file")?;
    serde_json::to_writer(BufWriter::new(file), &*guard)
      .map_err(|_| "Failed to write to file")?;

    Ok(())
  }
}
