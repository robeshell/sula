const SERVICE: &str = "sula.scraper-keys";

#[cfg(not(test))]
pub fn get(id: &str) -> anyhow::Result<String> {
    keyring::Entry::new(SERVICE, id)?.get_password().map_err(Into::into)
}
#[cfg(not(test))]
pub fn set(id: &str, value: &str) -> anyhow::Result<()> {
    keyring::Entry::new(SERVICE, id)?.set_password(value).map_err(Into::into)
}
#[cfg(not(test))]
pub fn remove(id: &str) {
    if let Ok(entry) = keyring::Entry::new(SERVICE, id) { let _ = entry.delete_credential(); }
}

#[cfg(test)]
fn values() -> &'static std::sync::Mutex<std::collections::HashMap<String,String>> {
    static VALUES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String,String>>> = std::sync::OnceLock::new();
    VALUES.get_or_init(Default::default)
}
#[cfg(test)]
pub fn get(id: &str) -> anyhow::Result<String> { values().lock().unwrap().get(id).cloned().ok_or_else(|| anyhow::anyhow!("missing test credential")) }
#[cfg(test)]
pub fn set(id: &str, value: &str) -> anyhow::Result<()> { let _ = SERVICE; values().lock().unwrap().insert(id.into(), value.into()); Ok(()) }
#[cfg(test)]
pub fn remove(id: &str) { values().lock().unwrap().remove(id); }
