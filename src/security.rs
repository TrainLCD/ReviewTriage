use anyhow::{Context, Result, ensure};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

// Defense in depth before saving model output or sending any issue payload.
// This deliberately rejects credential-looking content instead of logging it.
pub fn check_output(text: &str) -> Result<()> {
    let lower = text.to_ascii_lowercase();
    for pattern in [
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "-----begin",
        "private_key",
        "sk-proj-",
        "sk-svcacct-",
        "access_token",
        "refresh_token",
    ] {
        ensure!(
            !lower.contains(pattern),
            "credential-like content detected; output rejected"
        );
    }
    Ok(())
}

pub struct CodexProfile {
    pub home: PathBuf,
    pub config: PathBuf,
    pub codex: PathBuf,
    pub auth_store: String,
}

pub fn isolated_home(root: &Path) -> Result<CodexProfile> {
    let codex = env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".codex")))
        .context("HOME or CODEX_HOME is required for Codex authentication")?;
    let codex = match fs::canonicalize(codex) {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // API-key authentication does not require a pre-existing login profile.
            let path = root.join("codex");
            fs::create_dir(&path)?;
            path
        }
        Err(_) => anyhow::bail!("cannot access Codex authentication profile"),
    };
    // Keep Codex's native auth storage identity. Copying rotating refresh tokens creates
    // stale credentials; copying them back also races other Codex processes. Only Codex
    // reads/writes authentication. No configuration other than this enum is forwarded.
    let auth_store = match fs::read_to_string(codex.join("config.toml")) {
        Ok(text) => auth_store(&text)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => "file".into(),
        Err(_) => anyhow::bail!("cannot read Codex authentication storage setting"),
    };
    let home = root.join("home");
    let config = root.join("config");
    for p in [&home, &config] {
        fs::create_dir(p)?;
    }
    Ok(CodexProfile {
        home,
        config,
        codex,
        auth_store,
    })
}

fn auth_store(text: &str) -> Result<String> {
    // Do not include parser errors: config.toml can contain credentials.
    let config: toml::Value =
        toml::from_str(text).map_err(|_| anyhow::anyhow!("invalid Codex config.toml"))?;
    let store = match config.get("cli_auth_credentials_store") {
        None => "file",
        Some(v) => v
            .as_str()
            .context("invalid Codex authentication storage setting")?,
    };
    ensure!(
        matches!(store, "file" | "keyring" | "auto"),
        "unsupported Codex authentication storage setting"
    );
    Ok(store.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_known_auth_store_setting_is_forwarded() {
        assert_eq!(auth_store("model = 'ignored'").unwrap(), "file");
        for store in ["file", "auto", "keyring"] {
            assert_eq!(
                auth_store(&format!("cli_auth_credentials_store = '{store}'")).unwrap(),
                store
            );
        }
        assert!(auth_store("cli_auth_credentials_store = 'ephemeral'").is_err());
        assert!(auth_store("cli_auth_credentials_store = 1").is_err());
        let error = auth_store("secret-invalid-toml").unwrap_err().to_string();
        assert!(!error.contains("secret-invalid-toml"));
    }
    #[test]
    fn credentials_are_rejected_without_echoing_them() {
        for text in [
            "ghp_test",
            "gho_test",
            "github_pat_test",
            "-----BEGIN PRIVATE KEY-----",
            "\"private_key\":\"test\"",
            "refresh_token",
        ] {
            let e = check_output(text).unwrap_err().to_string();
            assert!(!e.contains(text));
        }
        assert!(check_output("駅名を修正する").is_ok());
    }
}
