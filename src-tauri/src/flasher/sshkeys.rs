// Command: SSH public keys (auto-detect + read a picked file)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub struct SshPublicKey {
    pub label: String,
    pub contents: String,
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// The operator's `~/.ssh/*.pub` public keys, so the UI can offer them instead
/// of making the user cat-and-paste. Best-effort: returns empty on any error.
#[tauri::command]
pub fn detect_ssh_keys() -> Vec<SshPublicKey> {
    match home_dir() {
        Some(home) => detect_ssh_keys_in(&home.join(".ssh")),
        None => Vec::new(),
    }
}

fn detect_ssh_keys_in(ssh: &std::path::Path) -> Vec<SshPublicKey> {
    let Ok(entries) = std::fs::read_dir(ssh) else {
        return Vec::new();
    };
    let mut keys: Vec<SshPublicKey> = entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("pub"))
        .filter_map(|e| {
            let contents = std::fs::read_to_string(e.path()).ok()?.trim().to_string();
            (!contents.is_empty()).then(|| SshPublicKey {
                label: e.file_name().to_string_lossy().to_string(),
                contents,
            })
        })
        .collect();
    keys.sort_by(|a, b| a.label.cmp(&b.label));
    keys
}

const MAX_PUBKEY_BYTES: u64 = 64 * 1024;

/// Reads a public-key file the user picked in the file dialog. Capped so a
/// misclick on a huge file can't be slurped into memory.
#[tauri::command]
pub fn read_public_key(path: String) -> Result<String, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("Cannot read key file: {e}"))?;
    if meta.len() > MAX_PUBKEY_BYTES {
        return Err("That file is too large to be an SSH public key.".to_string());
    }
    let contents =
        std::fs::read_to_string(&path).map_err(|e| format!("Cannot read key file: {e}"))?;
    Ok(contents.trim().to_string())
}

/// What a public key *is*, for a human about to disable password login with it.
/// The comment (`user@host`) is the part people recognise; the fingerprint is
/// what `ssh-keygen -lf` prints, so it can be compared against the key they
/// believe they hold. Returns `None` for anything that is not a key.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyIdentity {
    pub algorithm: String,
    pub comment: String,
    pub fingerprint: String,
}

#[tauri::command]
pub fn identify_public_key(key: String) -> Option<SshKeyIdentity> {
    use base64::Engine;
    use sha2::{Digest, Sha256};

    let mut parts = key.split_whitespace();
    let algorithm = parts.next()?.to_string();
    let blob = parts.next()?;
    let comment = parts.collect::<Vec<_>>().join(" ");

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .ok()?;
    if decoded.is_empty() {
        return None;
    }

    // OpenSSH prints the SHA256 digest base64'd with the padding stripped.
    let digest = Sha256::digest(&decoded);
    let encoded = base64::engine::general_purpose::STANDARD.encode(digest);

    Some(SshKeyIdentity {
        algorithm,
        comment,
        fingerprint: format!("SHA256:{}", encoded.trim_end_matches('=')),
    })
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flasher::isolated_dir;

    #[test]
    fn detect_ssh_keys_reads_pub_files_only() {
        let dir = isolated_dir("ssh-detect");
        std::fs::write(
            dir.join("id_ed25519.pub"),
            "  ssh-ed25519 AAAAKEY me@host  \n",
        )
        .unwrap();
        std::fs::write(dir.join("id_ed25519"), "PRIVATE KEY").unwrap();
        std::fs::write(dir.join("empty.pub"), "   \n").unwrap();

        let keys = detect_ssh_keys_in(&dir);

        assert_eq!(
            keys.len(),
            1,
            "only non-empty .pub files, never private keys"
        );
        assert_eq!(keys[0].label, "id_ed25519.pub");
        assert_eq!(keys[0].contents, "ssh-ed25519 AAAAKEY me@host");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn identifies_a_public_key_the_way_ssh_keygen_does() {
        // ssh-keygen -lf on this key prints exactly this fingerprint.
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ3z5r1sZ0K7DxHhqTn7lVXJHkiqrxKZ+i0Ck0KFqkkn operator@base";
        let id = identify_public_key(key.to_string()).expect("a key");
        assert_eq!(id.algorithm, "ssh-ed25519");
        assert_eq!(id.comment, "operator@base");
        // Verified against `ssh-keygen -lf` on this exact key.
        assert_eq!(
            id.fingerprint,
            "SHA256:6ToLTRFTRhCceqBA4tBy84TESMsJViBkYltFaKXXP20"
        );
    }

    #[test]
    fn a_key_with_no_comment_still_identifies() {
        let id = identify_public_key(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ3z5r1sZ0K7DxHhqTn7lVXJHkiqrxKZ+i0Ck0KFqkkn"
                .into(),
        )
        .expect("a key");
        assert_eq!(id.comment, "");
    }

    #[test]
    fn rubbish_is_not_a_key() {
        assert!(identify_public_key("hello".into()).is_none());
        assert!(identify_public_key("ssh-ed25519 not-base64!!".into()).is_none());
        assert!(identify_public_key(String::new()).is_none());
    }

    #[test]
    fn read_public_key_trims_and_caps_size() {
        let dir = isolated_dir("ssh-read");
        let ok = dir.join("k.pub");
        std::fs::write(&ok, "ssh-ed25519 AAAA me@host\n").unwrap();
        assert_eq!(
            read_public_key(ok.to_string_lossy().into()).unwrap(),
            "ssh-ed25519 AAAA me@host"
        );

        let big = dir.join("big.pub");
        std::fs::write(&big, vec![b'a'; (MAX_PUBKEY_BYTES + 1) as usize]).unwrap();
        assert!(read_public_key(big.to_string_lossy().into()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
