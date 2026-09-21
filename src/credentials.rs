//! Global credentials are kept separate from project config and report serialization.
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

type Keys = BTreeMap<String, String>;

pub fn path() -> Result<PathBuf> {
    Ok(crate::config::data_home()?.join("credentials.json"))
}
fn read(path: &Path) -> Result<Keys> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Keys::new()),
        Err(_) => bail!("Cannot read global credentials file {}", path.display()),
        Ok(metadata) if !metadata.file_type().is_file() => {
            bail!(
                "Global credentials must be a regular file: {}",
                path.display()
            );
        }
        Ok(_) => {}
    }
    let bytes = fs::read(path)
        .with_context(|| format!("Cannot read global credentials file {}", path.display()))?;
    // Do not include serde errors: malformed input could expose a key in an error message.
    serde_json::from_slice(&bytes).map_err(|_| {
        anyhow::anyhow!(
            "Invalid global credentials file {}; expected a JSON object of key names and values",
            path.display()
        )
    })
}
fn environment(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}
pub fn resolve(name: &str) -> Result<Option<String>> {
    if let Some(value) = environment(name) {
        return Ok(Some(value));
    }
    let Ok(path) = path() else {
        return Ok(None);
    };
    Ok(read(&path)?.remove(name).filter(|v| !v.trim().is_empty()))
}
pub fn required(name: &str) -> Result<String> {
    resolve(name)?.with_context(|| format!("Missing API key {name}. Run jta init to save it globally, or set {name} in your shell or project .env."))
}
fn save(path: &Path, name: &str, key: &str) -> Result<()> {
    let mut keys = read(path)?;
    keys.insert(name.to_owned(), key.to_owned());
    let parent = path.parent().context("Missing credentials directory")?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tmp.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    serde_json::to_writer_pretty(&mut tmp, &keys)?;
    tmp.write_all(b"\n")?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Only called by interactive setup; secrets are never accepted as CLI arguments.
pub fn prompt(name: &str, label: &str) -> Result<()> {
    let path = path()?;
    let stored = read(&path)?.remove(name).filter(|v| !v.trim().is_empty());
    if let Some(value) = environment(name).filter(|v| stored.as_ref() != Some(v)) {
        loop {
            eprint!("{label}: {name} is set in your shell or project .env. Save it globally for jta? [Y/n]: ");
            io::stderr().flush()?;
            let mut answer = String::new();
            if io::stdin().read_line(&mut answer)? == 0 {
                return Ok(());
            }
            match answer.trim().to_ascii_lowercase().as_str() {
                "" | "y" | "yes" => {
                    save(&path, name, &value)?;
                    eprintln!("Saved {label} key globally.");
                    return Ok(());
                }
                "n" | "no" => return Ok(()),
                _ => eprintln!(
                    "Enter y to save the current key, or n to leave it in your environment."
                ),
            }
        }
    }
    let action = if stored.is_some() {
        "keep saved key"
    } else {
        "skip"
    };
    let key = rpassword::prompt_password(format!("{label} API key ({name}; hidden; Enter to {action}): "))
        .context("Could not read API key from the terminal; use an environment variable or jta init --no-input")?;
    let key = key.trim();
    if !key.is_empty() {
        save(&path, name, key)?;
        eprintln!("Saved {label} key globally.");
        if environment(name).is_some() {
            eprintln!("The shell/project key still takes precedence until it is unset.");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_keys_atomically_preserving_other_key_names() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("jta/credentials.json");
        save(&path, "JEV_API_KEY", "test-jev").unwrap();
        save(&path, "CUSTOM_JEV_KEY", "test-custom").unwrap();
        save(&path, "JEV_API_KEY", "test-replacement").unwrap();
        let keys = read(&path).unwrap();
        assert_eq!(keys["JEV_API_KEY"], "test-replacement");
        assert_eq!(keys["CUSTOM_JEV_KEY"], "test-custom");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn malformed_keys_never_appear_in_errors_or_get_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("credentials.json");
        fs::write(&path, r#"{"JEV_API_KEY":{"secret-sentinel":42}}"#).unwrap();
        let error = read(&path).unwrap_err().to_string();
        assert!(!error.contains("secret-sentinel"));
        assert!(save(&path, "OTHER", "replacement").is_err());
        assert!(fs::read_to_string(path)
            .unwrap()
            .contains("secret-sentinel"));
    }
    #[test]
    #[cfg(unix)]
    fn rejects_symlink_credentials() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target");
        fs::write(&target, "{}").unwrap();
        let path = temp.path().join("credentials.json");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(save(&path, "KEY", "secret").is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "{}");
    }
}
