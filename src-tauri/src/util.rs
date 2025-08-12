use keyring::Entry;

pub fn load_secret(name: &str) -> Option<String> {
    Entry::new("notisr", name)
        .ok()?
        .get_secret()
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}