//! Port of the logins tests of `test/shell.test.js`: a login becomes the right headers for its site, the secret is
//! never shown nor written in clear, and a file left by the Electron app is brought over.
mod common;
use common::tmp_dir;
use std::path::Path;
use std::sync::Arc;
use tachy_core::logins::{Crypt, Logins, clean_host};

/// A stand-in for the Windows encryption: reversible, and clearly not the plain text.
struct Reverse;
impl Crypt for Reverse {
    fn encrypt(&self, text: &str) -> Result<String, String> {
        Ok(text.chars().rev().collect::<String>() + "#")
    }
    fn decrypt(&self, blob: &str) -> Result<String, String> {
        blob.strip_suffix('#')
            .map(|s| s.chars().rev().collect())
            .ok_or_else(|| "not ours".to_string())
    }
}

/// Like `Reverse`, and it knows an older format ("old:" in front) that has to be brought over.
struct WithLegacy;
impl Crypt for WithLegacy {
    fn encrypt(&self, text: &str) -> Result<String, String> {
        Reverse.encrypt(text)
    }
    fn decrypt(&self, blob: &str) -> Result<String, String> {
        match blob.strip_prefix("old:") {
            Some(rest) => Ok(rest.to_string()),
            None => Reverse.decrypt(blob),
        }
    }
    fn is_legacy(&self, blob: &str) -> bool {
        blob.starts_with("old:")
    }
}

struct Broken;
impl Crypt for Broken {
    fn encrypt(&self, text: &str) -> Result<String, String> {
        Reverse.encrypt(text)
    }
    fn decrypt(&self, _: &str) -> Result<String, String> {
        Err("cannot decrypt".into())
    }
}

fn logins(file: &Path) -> Logins {
    Logins::new(file, Arc::new(Reverse))
}

fn h(pairs: &[(&str, &str)]) -> Option<Vec<(String, String)>> {
    Some(
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect(),
    )
}

#[test]
fn a_site_is_read_out_of_an_address_a_bare_host_or_a_host_with_a_port() {
    assert_eq!(
        clean_host("https://Example.com/some/page?x=1").unwrap(),
        "example.com"
    );
    assert_eq!(clean_host("example.com").unwrap(), "example.com");
    assert_eq!(
        clean_host("files.example.com:8080").unwrap(),
        "files.example.com"
    );
    assert_eq!(clean_host("*.example.com").unwrap(), "example.com");
    assert!(clean_host("").unwrap_err().contains("site"));
    assert!(clean_host("exa mple").unwrap_err().contains("invalide"));
}

#[test]
fn logins_become_the_right_headers_for_that_site_and_its_subdomains_only() {
    let l = logins(&tmp_dir().join("logins.json"));
    l.save("https://example.com/login", "basic", "moi", "pa:ss")
        .unwrap();
    l.save("api.other.org", "bearer", "", "tok123").unwrap();
    l.save("cdn.net", "cookie", "", "sid=abc").unwrap();

    // "moi:pa:ss" in base64
    assert_eq!(
        l.headers_for("https://example.com/a.zip"),
        h(&[("Authorization", "Basic bW9pOnBhOnNz")])
    );
    assert!(
        l.headers_for("https://files.example.com/a.zip").is_some(),
        "a subdomain is covered"
    );
    assert_eq!(
        l.headers_for("https://notexample.com/a.zip"),
        None,
        "a lookalike is not"
    );
    assert_eq!(l.headers_for("https://example.com.evil.io/a.zip"), None);
    assert_eq!(
        l.headers_for("https://api.other.org/x"),
        h(&[("Authorization", "Bearer tok123")])
    );
    assert_eq!(
        l.headers_for("http://cdn.net/x"),
        h(&[("Cookie", "sid=abc")])
    );
    assert_eq!(l.headers_for("not a url"), None);

    // the more specific entry wins
    l.save("vip.example.com", "bearer", "", "vip").unwrap();
    assert_eq!(
        l.headers_for("https://vip.example.com/x"),
        h(&[("Authorization", "Bearer vip")])
    );
    assert!(
        l.headers_for("https://example.com/x").unwrap()[0]
            .1
            .starts_with("Basic")
    );
}

#[test]
fn the_list_never_shows_a_secret_and_the_file_on_disk_holds_none_in_clear() {
    let file = tmp_dir().join("logins.json");
    let l = logins(&file);
    let after = l
        .save("example.com", "basic", "moi", "motdepasse-unique")
        .unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(
        (
            after[0].host.as_str(),
            after[0].kind.as_str(),
            after[0].user.as_str()
        ),
        ("example.com", "basic", "moi")
    );
    assert!(
        !std::fs::read_to_string(&file)
            .unwrap()
            .contains("motdepasse-unique")
    );

    // a fresh start reads the same entries back
    let again = logins(&file);
    assert_eq!(again.list().len(), 1);
    assert!(again.headers_for("https://example.com/x").is_some());

    // saving the same site again replaces it; removing works
    again.save("example.com", "bearer", "", "new").unwrap();
    let now = again.list();
    assert_eq!(
        (now.len(), now[0].kind.as_str(), now[0].user.as_str()),
        (1, "bearer", "")
    );
    assert!(again.remove("https://example.com").unwrap().is_empty());
    assert_eq!(again.headers_for("https://example.com/x"), None);
}

#[test]
fn incomplete_or_unknown_logins_are_refused() {
    let l = logins(&tmp_dir().join("l.json"));
    let err = |r: Result<_, String>| -> String { r.map(|_: Vec<_>| ()).unwrap_err() };
    assert!(err(l.save("a.com", "basic", "", "x")).contains("utilisateur"));
    assert!(err(l.save("a.com", "basic", "u", "")).contains("mot de passe"));
    assert!(err(l.save("a.com", "cookie", "", "")).contains("cookie"));
    assert!(err(l.save("a.com", "bearer", "", "")).contains("jeton"));
    assert!(err(l.save("a.com", "magic", "", "x")).contains("inconnu"));
    assert!(err(l.save("", "bearer", "", "x")).contains("site"));
    assert!(err(l.save("a.com", "bearer", "", &"x".repeat(4097))).contains("trop longue"));
    assert!(l.list().is_empty());
}

#[test]
fn a_secret_that_cannot_be_decrypted_simply_gives_no_header() {
    let file = tmp_dir().join("l.json");
    logins(&file).save("a.com", "bearer", "", "x").unwrap();
    let broken = Logins::new(&file, Arc::new(Broken));
    assert_eq!(broken.headers_for("https://a.com/f"), None);
    assert_eq!(
        broken.list().len(),
        1,
        "the entry is still listed so it can be removed"
    );
}

#[test]
fn a_damaged_logins_file_starts_empty_instead_of_crashing() {
    let file = tmp_dir().join("l.json");
    std::fs::write(&file, "{ not json").unwrap();
    assert!(logins(&file).list().is_empty());
}

#[test]
fn entries_left_by_the_electron_app_are_brought_over_and_the_old_file_is_kept_aside() {
    let dir = tmp_dir();
    let file = dir.join("logins.json");
    let old = r#"[{"host":"a.com","type":"bearer","user":"","secret":"old:tok-a"},
                  {"host":"b.com","type":"basic","user":"moi","secret":"old:pass-b"},
                  {"host":"c.com","type":"cookie","user":"","secret":"nzis=c#"}]"#;
    std::fs::write(&file, old).unwrap();

    let l = Logins::new(&file, Arc::new(WithLegacy));
    assert_eq!(
        l.headers_for("https://a.com/x"),
        h(&[("Authorization", "Bearer tok-a")])
    );
    assert!(l.headers_for("https://b.com/x").is_some());
    let backup = dir.join("logins.json.electron.bak");
    assert_eq!(
        std::fs::read_to_string(&backup).unwrap(),
        old,
        "the old file is kept as it was"
    );
    let now = std::fs::read_to_string(&file).unwrap();
    assert!(!now.contains("old:"), "nothing is left in the old format");
    assert!(!now.contains("tok-a"));

    // a second start has nothing to bring over and leaves the backup alone
    std::fs::write(&backup, "kept").unwrap();
    let again = Logins::new(&file, Arc::new(WithLegacy));
    assert_eq!(again.list().len(), 3);
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), "kept");
}
