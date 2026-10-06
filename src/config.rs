//! Where the Worker URL and read token live between launches.
//!
//! The URL is not secret and sits in a JSON file under Application Support so
//! it is easy to inspect or delete. The token goes only to the macOS Keychain;
//! it is never written to the config file, so copying or syncing that file
//! cannot leak it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::api::Secret;

pub const APP_DIR: &str = "music-warehouse-gui";
const CONFIG_FILE: &str = "config.json";
const KEYCHAIN_SERVICE: &str = "music-warehouse-gui";
const KEYCHAIN_ACCOUNT: &str = "READ_TOKEN";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub worker_url: String,
}

/// `~/Library/Application Support/music-warehouse-gui`.
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(APP_DIR))
}

pub fn load_from(dir: &Path) -> io::Result<Option<Config>> {
    let path = dir.join(CONFIG_FILE);
    match fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn save_to(dir: &Path, config: &Config) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let text = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    // Write then rename, so a crash mid-save never leaves a truncated file
    // that would bounce the user back to first-run setup.
    let tmp = dir.join(format!("{CONFIG_FILE}.tmp"));
    fs::write(&tmp, text)?;
    fs::rename(tmp, dir.join(CONFIG_FILE))
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|e| format!("Keychain unavailable: {e}"))
}

/// `Ok(None)` means no token has been saved yet, which is first-run, not an error.
pub fn load_token() -> Result<Option<Secret>, String> {
    #[cfg(feature = "dev-capture")]
    if let Some(token) = dev_token_from_file()? {
        return Ok(Some(token));
    }
    match entry()?.get_password() {
        Ok(value) => Ok(Some(Secret::new(value))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read the token from the Keychain: {e}")),
    }
}

/// Development builds only. Every rebuild of an ad-hoc-signed binary makes
/// macOS ask again before releasing the Keychain item, which stalls automated
/// runs. `MWGUI_DEV_TOKEN_FILE` points at a dotenv file (such as the Worker's
/// `.dev.vars`); only its `READ_TOKEN=` line is read, so an admin token in the
/// same file is never picked up.
#[cfg(feature = "dev-capture")]
fn dev_token_from_file() -> Result<Option<Secret>, String> {
    let Some(path) = std::env::var_os("MWGUI_DEV_TOKEN_FILE") else {
        return Ok(None);
    };
    let text = fs::read_to_string(&path).map_err(|e| format!("MWGUI_DEV_TOKEN_FILE: {e}"))?;
    Ok(read_token_line(&text))
}

#[cfg_attr(not(any(test, feature = "dev-capture")), allow(dead_code))]
fn read_token_line(dotenv: &str) -> Option<Secret> {
    dotenv
        .lines()
        .filter_map(|line| line.trim().strip_prefix("READ_TOKEN="))
        .map(|value| Secret::new(value.trim_matches('"')))
        .find(|token| !token.is_empty())
}

pub fn save_token(token: &Secret) -> Result<(), String> {
    entry()?
        .set_password(token.expose())
        .map_err(|e| format!("Could not save the token to the Keychain: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mwgui-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn dev_token_file_only_yields_the_read_token() {
        let text = "# comment\nADMIN_TOKEN=admin-value\nREAD_TOKEN=\"read-value\"\n";
        assert_eq!(read_token_line(text).unwrap().expose(), "read-value");
        assert!(read_token_line("ADMIN_TOKEN=admin-value\n").is_none());
    }

    #[test]
    fn missing_config_is_first_run() {
        let dir = scratch_dir("missing");
        assert_eq!(load_from(&dir).unwrap(), None);
    }

    #[test]
    fn config_round_trips_without_a_token_field() {
        let dir = scratch_dir("roundtrip");
        let config = Config {
            worker_url: "https://music.example.com".into(),
        };
        save_to(&dir, &config).unwrap();
        assert_eq!(load_from(&dir).unwrap(), Some(config));
        let raw = fs::read_to_string(dir.join(CONFIG_FILE)).unwrap();
        assert!(!raw.to_lowercase().contains("token"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn corrupt_config_is_an_error_not_first_run() {
        let dir = scratch_dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(CONFIG_FILE), "{not json").unwrap();
        assert!(load_from(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
}
