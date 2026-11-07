//! Shared token-file loading for the fleet-queue brokers and client
//! (#6320).
//!
//! Invariant: a loaded token is never empty. Before #6320 every load
//! site read the file and trimmed it with no empty check, so an empty
//! or whitespace-only token file became the expected token `""` — and
//! any client sending an empty `X-Agent-Token` header authenticated,
//! including to the secrets-broker's secret-value endpoint. A missing
//! or unreadable file already failed loudly; an empty one failed open.
//!
//! Oracle for the tests below: the expected value is exactly the
//! file's trimmed contents, and the empty set is {empty file,
//! whitespace-only file} — both must be rejected with an error that
//! names the token file, so a broker refuses to start (its caller
//! exits 3, the same fail-loud shape as an unreadable file) and the
//! client refuses to send.

/// Read and trim a token file, rejecting an empty or whitespace-only
/// token. Every error names the token file.
pub fn load_token_file(token_file: &str) -> Result<String, String> {
    let raw = std::fs::read_to_string(token_file)
        .map_err(|e| format!("cannot read token file {token_file}: {e}"))?;
    let token = raw.trim();
    if token.is_empty() {
        return Err(format!(
            "token file {token_file} is empty or whitespace-only; refusing to use an empty token"
        ));
    }
    Ok(token.to_string())
}

#[cfg(test)]
mod tests {
    use super::load_token_file;

    fn write_fixture(name: &str, contents: &str) -> String {
        let path = std::env::temp_dir().join(format!(
            "fleet-queue-token-file-test-{}-{name}",
            std::process::id()
        ));
        std::fs::write(&path, contents).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn empty_token_file_is_rejected_and_the_error_names_the_file() {
        let path = write_fixture("empty", "");
        let err = load_token_file(&path).expect_err("empty token file must be rejected");
        assert!(err.contains(&path), "error names the file: {err}");
        assert!(err.contains("empty"), "error says why: {err}");
    }

    #[test]
    fn whitespace_only_token_file_is_rejected_and_the_error_names_the_file() {
        let path = write_fixture("whitespace", "  \n\t \r\n");
        let err = load_token_file(&path).expect_err("whitespace-only token file must be rejected");
        assert!(err.contains(&path), "error names the file: {err}");
        assert!(err.contains("empty"), "error says why: {err}");
    }

    #[test]
    fn non_empty_token_file_loads_trimmed() {
        let path = write_fixture("real", "  fixture-token-value\n");
        assert_eq!(load_token_file(&path).unwrap(), "fixture-token-value");
    }

    #[test]
    fn missing_token_file_is_rejected_and_the_error_names_the_file() {
        let path = std::env::temp_dir()
            .join(format!(
                "fleet-queue-token-file-test-{}-missing",
                std::process::id()
            ))
            .to_string_lossy()
            .into_owned();
        let err = load_token_file(&path).expect_err("missing token file must be rejected");
        assert!(err.contains(&path), "error names the file: {err}");
    }
}
