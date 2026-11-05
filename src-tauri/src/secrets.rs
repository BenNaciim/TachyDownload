//! Encryption of the site logins with the Windows account (DPAPI), and the bridge from the Electron app: its
//! `safeStorage` kept `"v10"` + 12-byte nonce + AES-256-GCM ciphertext + tag, with the 32-byte key in `Local State`
//! (`os_crypt.encrypted_key` = base64(`"DPAPI"` + the key protected by DPAPI)).

use std::path::PathBuf;
use std::sync::Mutex;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tachy_core::logins::Crypt;

const NOT_AVAILABLE: &str = "Le chiffrement n’est pas disponible sur cet ordinateur";

#[cfg(windows)]
mod dpapi {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: the system allocated `cbData` bytes at `pbData`; they are copied, then released with LocalFree.
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        unsafe { LocalFree(Some(HLOCAL(out.pbData as *mut _))) };
        v
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(data);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: `input` points at `data`, which outlives the call; `out` is filled by the system.
        unsafe { CryptProtectData(&input, None, None, None, None, 0, &mut out) }
            .map_err(|e| e.to_string())?;
        Ok(take(out))
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(data);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: same as `protect`.
        unsafe { CryptUnprotectData(&input, None, None, None, None, 0, &mut out) }
            .map_err(|e| e.to_string())?;
        Ok(take(out))
    }
}

#[cfg(not(windows))]
mod dpapi {
    pub fn protect(_: &[u8]) -> Result<Vec<u8>, String> {
        Err(super::NOT_AVAILABLE.into())
    }
    pub fn unprotect(_: &[u8]) -> Result<Vec<u8>, String> {
        Err(super::NOT_AVAILABLE.into())
    }
}

pub struct Dpapi {
    /// The Electron app's `Local State`, where the key of its old secrets is.
    local_state: PathBuf,
    old_key: Mutex<Option<Vec<u8>>>,
}

impl Dpapi {
    pub fn new(local_state: PathBuf) -> Dpapi {
        Dpapi {
            local_state,
            old_key: Mutex::new(None),
        }
    }

    fn old_key(&self) -> Result<Vec<u8>, String> {
        let mut cached = self.old_key.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(k) = cached.as_ref() {
            return Ok(k.clone());
        }
        let state: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&self.local_state).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let wrapped = STANDARD
            .decode(
                state["os_crypt"]["encrypted_key"]
                    .as_str()
                    .ok_or("no key")?,
            )
            .map_err(|e| e.to_string())?;
        let key = dpapi::unprotect(
            wrapped
                .strip_prefix(b"DPAPI")
                .ok_or("unknown key wrapper")?,
        )?;
        *cached = Some(key.clone());
        Ok(key)
    }

    fn decrypt_old(&self, raw: &[u8]) -> Result<String, String> {
        let body = raw.strip_prefix(b"v10").ok_or("unknown format")?;
        if body.len() < 12 + 16 {
            return Err("truncated".into());
        }
        let (nonce, data) = body.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(&self.old_key()?).map_err(|e| e.to_string())?;
        let plain = cipher
            .decrypt(Nonce::from_slice(nonce), data)
            .map_err(|e| e.to_string())?;
        String::from_utf8(plain).map_err(|e| e.to_string())
    }
}

impl Crypt for Dpapi {
    fn encrypt(&self, text: &str) -> Result<String, String> {
        dpapi::protect(text.as_bytes())
            .map(|b| STANDARD.encode(b))
            .map_err(|_| NOT_AVAILABLE.to_string())
    }

    fn decrypt(&self, blob: &str) -> Result<String, String> {
        let raw = STANDARD.decode(blob).map_err(|e| e.to_string())?;
        if raw.starts_with(b"v10") {
            return self.decrypt_old(&raw);
        }
        String::from_utf8(dpapi::unprotect(&raw)?).map_err(|e| e.to_string())
    }

    fn is_legacy(&self, blob: &str) -> bool {
        STANDARD
            .decode(blob)
            .is_ok_and(|raw| raw.starts_with(b"v10"))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tachy_core::logins::Logins;

    fn work_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tachy-secrets-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_secret_survives_a_round_trip_and_is_not_readable_as_saved() {
        let d = Dpapi::new(work_dir("round").join("Local State"));
        let blob = d.encrypt("mot de passe é€ 日本").unwrap();
        assert!(!blob.contains("mot de passe"));
        assert_eq!(d.decrypt(&blob).unwrap(), "mot de passe é€ 日本");
        assert!(!d.is_legacy(&blob));
    }

    #[test]
    fn what_the_electron_app_saved_is_read_and_saved_again_the_new_way() {
        let dir = work_dir("old");
        // what safeStorage wrote: a key protected by DPAPI in `Local State`, and v10 secrets under that key
        let key = [7u8; 32];
        let wrapped = [b"DPAPI".as_slice(), &dpapi::protect(&key).unwrap()].concat();
        std::fs::write(
            dir.join("Local State"),
            serde_json::json!({ "os_crypt": { "encrypted_key": STANDARD.encode(wrapped) } })
                .to_string(),
        )
        .unwrap();
        let nonce = [3u8; 12];
        let sealed = Aes256Gcm::new_from_slice(&key)
            .unwrap()
            .encrypt(Nonce::from_slice(&nonce), "tok-électron".as_bytes())
            .unwrap();
        let old = STANDARD.encode([b"v10".as_slice(), &nonce, &sealed].concat());
        let file = dir.join("logins.json");
        std::fs::write(
            &file,
            serde_json::json!([{ "host": "a.com", "type": "bearer", "user": "", "secret": old }])
                .to_string(),
        )
        .unwrap();

        let logins = Logins::new(&file, Arc::new(Dpapi::new(dir.join("Local State"))));
        assert_eq!(
            logins.headers_for("https://a.com/f").unwrap(),
            vec![(
                "Authorization".to_string(),
                "Bearer tok-électron".to_string()
            )]
        );
        assert!(dir.join("logins.json.electron.bak").exists());
        let now = std::fs::read_to_string(&file).unwrap();
        assert!(!now.contains(&old), "the entry now holds a DPAPI blob");
        // and it reads back from the file alone, with no `Local State` any more
        std::fs::remove_file(dir.join("Local State")).unwrap();
        let again = Logins::new(&file, Arc::new(Dpapi::new(dir.join("Local State"))));
        assert!(again.headers_for("https://a.com/f").is_some());
    }
}
