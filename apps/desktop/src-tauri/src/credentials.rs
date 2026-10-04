//! Secure credential storage in the OS keychain (macOS Keychain, Windows
//! Credential Manager, Linux Secret Service) via the `keyring` crate. Secrets
//! never touch disk in plaintext and are never logged.

pub const SERVICE: &str = "ai.fxvps.desktop";

fn entry(account: &str) -> keyring::Result<keyring::Entry> {
    if account.is_empty() || account.len() > 128 {
        return Err(keyring::Error::Invalid(
            "account".into(),
            "must be 1..=128 bytes".into(),
        ));
    }
    keyring::Entry::new(SERVICE, account)
}

pub fn set(account: &str, secret: &str) -> keyring::Result<()> {
    entry(account)?.set_password(secret)
}

pub fn get(account: &str) -> keyring::Result<Option<String>> {
    match entry(account)?.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn delete(account: &str) -> keyring::Result<()> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_account_names() {
        assert!(entry("").is_err());
        assert!(entry(&"a".repeat(129)).is_err());
    }
}
