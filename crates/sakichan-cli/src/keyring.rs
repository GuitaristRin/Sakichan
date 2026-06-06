use anyhow::{Context, Result};
use keyring::Entry;

const SERVICE_NAME: &str = "com.sakichan";

/// Store an API key in the system keychain.
pub fn set_api_key(model_id: &str, api_key: &str) -> Result<()> {
    let entry = Entry::new(SERVICE_NAME, model_id)?;
    entry.set_password(api_key)?;
    Ok(())
}

/// Retrieve an API key from the system keychain.
pub fn get_api_key(model_id: &str) -> Result<Option<String>> {
    let entry = Entry::new(SERVICE_NAME, model_id)?;
    match entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Delete an API key from the system keychain.
pub fn delete_api_key(model_id: &str) -> Result<bool> {
    let entry = Entry::new(SERVICE_NAME, model_id)?;
    match entry.delete_password() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(e.into()),
    }
}

/// Clear all known Sakichan API keys from the system keychain.
/// Returns the number of keys deleted.
pub fn clear_all_api_keys() -> Result<usize> {
    let known_models = ["sensenova", "deepseek"];
    let mut count = 0;
    for model_id in &known_models {
        if delete_api_key(model_id)? {
            count += 1;
        }
    }
    Ok(count)
}

/// Store the same API key for all known Sakichan models.
pub fn set_all_api_keys(api_key: &str) -> Result<usize> {
    let known_models = ["sensenova", "deepseek"];
    let mut count = 0;
    for model_id in &known_models {
        set_api_key(model_id, api_key)?;
        count += 1;
    }
    Ok(count)
}

/// Resolve an "ENC_KEYRING:..." config value to the actual API key.
/// Prompts the user if the key is not yet stored.
pub fn resolve_keyring_value(
    raw_value: &str,
    model_label: &str,
) -> Result<String> {
    if let Some(keyring_path) = raw_value.strip_prefix("ENC_KEYRING:") {
        // Format: "ENC_KEYRING:com.sakichan/sensenova" — extract the model-id part after '/'
        let model_id = keyring_path.split('/').next_back().unwrap_or(keyring_path);

        if let Some(key) = get_api_key(model_id)? {
            return Ok(key);
        }

        // Key not found, prompt user
        eprintln!(
            "🔑 API key for '{}' ({}) not found in system keychain.",
            model_label, model_id
        );
        eprint!("Please enter your API key: ");
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .context("Failed to read API key input")?;
        let key = input.trim().to_string();

        if key.is_empty() {
            anyhow::bail!("API key cannot be empty");
        }

        set_api_key(model_id, &key)?;
        eprintln!("✅ API key saved to system keychain.");
        Ok(key)
    } else {
        // Not a keyring reference, return as-is
        Ok(raw_value.to_string())
    }
}