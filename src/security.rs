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

pub fn isolated_home(root: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let original = env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".codex")));
    let home = root.join("home");
    let config = root.join("config");
    let codex = root.join("codex");
    for p in [&home, &config, &codex] {
        fs::create_dir(p)?;
    }
    if let Some(auth) = original
        .map(|p| p.join("auth.json"))
        .filter(|p| p.is_file())
    {
        let dest = codex.join("auth.json");
        fs::copy(auth, &dest).context("copy Codex authentication into isolated profile")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(dest, fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok((home, config, codex))
}

#[cfg(test)]
mod tests {
    use super::*;
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
