//! App JWTs (RS256). The private key reaches `openssl dgst` on an inherited
//! pipe fd, never a file, argv or the environment; only the signature
//! touches it. The same construction as the gh domain's minter.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn app_jwt(app_id: &str, key_pem: &str) -> Result<String, String> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock before epoch".to_string())?
        .as_secs();
    let header = base64url(br#"{"alg":"RS256","typ":"JWT"}"#);
    let payload = base64url(
        format!(
            "{{\"iat\":{},\"exp\":{},\"iss\":\"{}\"}}",
            now.saturating_sub(60),
            now + 540,
            app_id
        )
        .as_bytes(),
    );
    let signing_input = format!("{header}.{payload}");
    let signature = rs256_sign(key_pem.as_bytes(), signing_input.as_bytes())?;
    Ok(format!("{signing_input}.{}", base64url(&signature)))
}

fn rs256_sign(key_pem: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    use std::os::unix::io::AsRawFd;
    use std::os::unix::process::CommandExt;
    let (reader, mut writer) = std::io::pipe().map_err(|e| format!("openssl sign: pipe: {e}"))?;
    // The key is ~1.7 KB, far under the pipe buffer, so this write
    // completes before the child starts and cannot deadlock.
    writer
        .write_all(key_pem)
        .map_err(|_| "openssl sign: key pipe write failed".to_string())?;
    drop(writer);
    let key_arg = format!("/dev/fd/{}", reader.as_raw_fd());
    let mut cmd = Command::new("openssl");
    cmd.args(["dgst", "-sha256", "-sign", &key_arg])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let fd = reader.as_raw_fd();
    // SAFETY: runs in the forked child before exec; only clears CLOEXEC on
    // the key pipe so `openssl` can open it through /dev/fd.
    unsafe {
        cmd.pre_exec(move || {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(|e| format!("openssl sign: spawn failed: {e}"))?;
    drop(reader);
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(data);
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("openssl sign: wait failed: {e}"))?;
    if !out.status.success() {
        return Err("openssl sign: signing failed (key shape?)".to_string());
    }
    Ok(out.stdout)
}

/// base64url without padding (RFC 7515).
pub fn base64url(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..=chunk.len() {
            out.push(ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize] as char);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_rfc4648_vectors() {
        assert_eq!(base64url(b""), "");
        assert_eq!(base64url(b"f"), "Zg");
        assert_eq!(base64url(b"fo"), "Zm8");
        assert_eq!(base64url(b"foo"), "Zm9v");
        assert_eq!(base64url(b"foob"), "Zm9vYg");
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
    }
}
