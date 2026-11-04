//! THROWAWAY (proof S3). Reads a data dir produced by `make-secret.js` (run under Electron) and decrypts every secret.
//!
//! Electron on Windows stores `safeStorage` data as: `"v10"` + 12-byte nonce + AES-256-GCM ciphertext + 16-byte tag.
//! The 32-byte key is in `<dir>\Local State` -> `os_crypt.encrypted_key` = base64(`"DPAPI"` + CryptProtectData(key)).
//! Also checks our own future format: plain DPAPI protect/unprotect of a secret.
//!
//!   spike-safestorage-read <data dir>

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
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
    let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
    unsafe { LocalFree(Some(HLOCAL(out.pbData as *mut _))) };
    v
}

fn dpapi_unprotect(data: &[u8]) -> Vec<u8> {
    let input = blob(data);
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(&input, None, None, None, None, 0, &mut out).expect("CryptUnprotectData")
    };
    take(out)
}

fn dpapi_protect(data: &[u8]) -> Vec<u8> {
    let input = blob(data);
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(&input, None, None, None, None, 0, &mut out).expect("CryptProtectData")
    };
    take(out)
}

fn main() {
    let dir = std::path::PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("usage: spike-safestorage-read <data dir>"),
    );
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("Local State")).expect("Local State"))
            .unwrap();
    let wrapped = STANDARD
        .decode(
            state["os_crypt"]["encrypted_key"]
                .as_str()
                .expect("os_crypt.encrypted_key"),
        )
        .unwrap();
    assert_eq!(&wrapped[..5], b"DPAPI", "unexpected key wrapper");
    let key = dpapi_unprotect(&wrapped[5..]);
    assert_eq!(key.len(), 32, "the key must be 32 bytes");
    let cipher = Aes256Gcm::new_from_slice(&key).unwrap();

    let file: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("spike-secrets.json")).unwrap()).unwrap();
    assert_eq!(
        file["available"], true,
        "safeStorage was not available under Electron"
    );
    let secrets = file["secrets"].as_array().unwrap();
    assert!(!secrets.is_empty());
    for s in secrets {
        let raw = STANDARD.decode(s["secret"].as_str().unwrap()).unwrap();
        assert_eq!(&raw[..3], b"v10", "unexpected version prefix");
        let (nonce, body) = raw[3..].split_at(12);
        let plain = cipher
            .decrypt(Nonce::from_slice(nonce), body)
            .expect("AES-GCM decrypt");
        let expected = s["plain"].as_str().unwrap();
        assert_eq!(String::from_utf8(plain).unwrap(), expected);
        println!(
            "ok  v10 secret of {} bytes decrypted and equal to the original",
            expected.len()
        );
    }

    // Our own format for the Rust app: DPAPI directly (same Windows account, no Electron).
    let mine = dpapi_protect("mot de passe é€ 日本".as_bytes());
    assert_eq!(
        String::from_utf8(dpapi_unprotect(&mine)).unwrap(),
        "mot de passe é€ 日本"
    );
    println!("ok  DPAPI round trip ({} bytes protected)", mine.len());
}
