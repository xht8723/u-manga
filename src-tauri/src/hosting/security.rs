//! Network authentication is separate from provider credentials and processing settings.
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::http::{HeaderMap, header};
use parking_lot::Mutex;
use std::collections::HashSet;
use std::{
    collections::HashMap,
    net::Ipv4Addr,
    time::{Duration, Instant},
};

pub const SESSION_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
pub const COOKIE: &str = "umanga_session";
const MAX_SESSIONS: usize = 64;

pub fn private_peer(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || (a == 100 && (64..=127).contains(&b))
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        (8..=1024).contains(&password.chars().count()),
        "Use a hosting password of 8–1024 characters."
    );
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map_err(|_| anyhow::anyhow!("Could not secure the hosting password."))?
        .to_string())
}
pub fn verify_password(hash: &str, password: &str) -> bool {
    password.len() <= 4096
        && valid_hash(hash)
        && PasswordHash::new(hash).is_ok_and(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
}
pub fn valid_hash(value: &str) -> bool {
    value.len() <= 256
        && PasswordHash::new(value).is_ok_and(|h| {
            h.algorithm.as_str() == "argon2id"
                && h.version == Some(19)
                && h.params.get_decimal("m") == Some(19456)
                && h.params.get_decimal("t") == Some(2)
                && h.params.get_decimal("p") == Some(1)
                && h.hash.is_some_and(|v| v.len() == 32)
                && h.salt.is_some()
        })
}
/// The authority comes from our listeners, never forwarded proxy headers.
pub fn request_site(headers: &HeaderMap, hosts: &HashSet<String>) -> Result<(), &'static str> {
    let authority = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !hosts.contains(&authority) {
        return Err("Use one of the hosting addresses shown on the PC.");
    }
    if let Some(origin) = headers.get(header::ORIGIN)
        && origin.to_str().ok() != Some(format!("http://{authority}").as_str())
    {
        return Err("This request came from a different site.");
    }
    if headers
        .get("sec-fetch-site")
        .is_some_and(|v| v == "cross-site")
    {
        return Err("This request came from a different site.");
    }
    Ok(())
}
fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[derive(Clone)]
pub struct Session {
    pub csrf: String,
    pub expires: Instant,
}
#[derive(Default)]
pub struct Sessions {
    entries: Mutex<HashMap<String, Session>>,
}
impl Sessions {
    pub fn create(&self, now: Instant) -> Option<(String, Session)> {
        let mut entries = self.entries.lock();
        entries.retain(|_, s| s.expires > now);
        if entries.len() >= MAX_SESSIONS {
            return None;
        }
        let token = random_token();
        let session = Session {
            csrf: random_token(),
            expires: now + SESSION_LIFETIME,
        };
        entries.insert(token.clone(), session.clone());
        Some((token, session))
    }
    pub fn get(&self, token: &str, now: Instant) -> Option<Session> {
        let mut entries = self.entries.lock();
        if entries.get(token).is_some_and(|s| s.expires <= now) {
            entries.remove(token);
        }
        entries.get(token).cloned()
    }
    pub fn remove(&self, token: &str) {
        self.entries.lock().remove(token);
    }
    pub fn count(&self) -> usize {
        let mut e = self.entries.lock();
        e.retain(|_, s| s.expires > Instant::now());
        e.len()
    }
    pub fn clear(&self) {
        self.entries.lock().clear();
    }
}

/// Bounded per-peer budget plus a global budget prevents address churn/hash starvation.
#[derive(Default)]
pub struct LoginBudget {
    entries: Mutex<HashMap<Ipv4Addr, (Instant, u32)>>,
    global: Mutex<Option<(Instant, u32)>>,
}
impl LoginBudget {
    pub fn take(&self, ip: Ipv4Addr, now: Instant) -> bool {
        let mut global = self.global.lock();
        let (started, count) = global.get_or_insert((now, 0));
        if now.duration_since(*started) >= Duration::from_secs(60) {
            *started = now;
            *count = 0;
        }
        if *count >= 60 {
            return false;
        }
        *count += 1;
        let mut entries = self.entries.lock();
        entries.retain(|_, (t, _)| now.duration_since(*t) < Duration::from_secs(60));
        if entries.len() >= 256 && !entries.contains_key(&ip) {
            return false;
        }
        let (_, count) = entries.entry(ip).or_insert((now, 0));
        if *count >= 5 {
            return false;
        }
        *count += 1;
        true
    }
}

pub fn cookie_token(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == COOKIE && value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| value.to_owned())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn network_scope_excludes_public_and_special_peers() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.4.5",
            "192.168.1.7",
            "100.64.1.9",
            "169.254.1.3",
        ] {
            assert!(private_peer(ip.parse().unwrap()));
        }
        for ip in [
            "8.8.8.8",
            "0.0.0.0",
            "100.63.1.1",
            "100.128.1.1",
            "172.15.1.1",
            "224.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!private_peer(ip.parse().unwrap()));
        }
    }
    #[test]
    fn sessions_are_unique_expire_and_revoke_without_wall_clock() {
        let sessions = Sessions::default();
        let now = Instant::now();
        let (a, sa) = sessions.create(now).unwrap();
        let (b, sb) = sessions.create(now).unwrap();
        assert_ne!(a, b);
        assert_ne!(sa.csrf, sb.csrf);
        assert_ne!(a, sa.csrf);
        assert!(sessions.get(&a, now + SESSION_LIFETIME).is_none());
        sessions.clear();
        assert!(sessions.get(&b, now).is_none());
    }
    #[test]
    fn login_attempts_are_bounded_and_recover() {
        let b = LoginBudget::default();
        let now = Instant::now();
        let ip = "127.0.0.1".parse().unwrap();
        for _ in 0..5 {
            assert!(b.take(ip, now));
        }
        assert!(!b.take(ip, now));
        assert!(b.take(ip, now + Duration::from_secs(60)));
    }
    #[test]
    fn password_is_not_plaintext_and_invalid_passwords_fail() {
        assert!(hash_password("short").is_err());
        let h = hash_password("地方password!").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(!h.contains("password!"));
        assert!(verify_password(&h, "地方password!"));
        assert!(!verify_password(&h, "wrong"));
        assert!(!verify_password("invalid", "地方password!"));
        assert!(!valid_hash(&h.replace("m=19456", "m=999999999")));
    }
    #[test]
    fn host_origin_and_proxy_headers_cannot_expand_scope() {
        let hosts = ["localhost:8080".into(), "192.168.1.2:8080".into()]
            .into_iter()
            .collect();
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "localhost:8080".parse().unwrap());
        assert!(request_site(&h, &hosts).is_ok());
        h.insert(header::ORIGIN, "http://localhost:8080".parse().unwrap());
        assert!(request_site(&h, &hosts).is_ok());
        for origin in [
            "null",
            "https://localhost:8080",
            "http://attacker.invalid",
            "http://localhost:8081",
        ] {
            h.insert(header::ORIGIN, origin.parse().unwrap());
            assert!(request_site(&h, &hosts).is_err());
        }
        h.remove(header::ORIGIN);
        h.insert(header::HOST, "attacker.invalid".parse().unwrap());
        h.insert("x-forwarded-host", "localhost:8080".parse().unwrap());
        assert!(request_site(&h, &hosts).is_err());
        h.insert(header::HOST, "LOCALHOST:8080".parse().unwrap());
        assert!(request_site(&h, &hosts).is_ok());
        h.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(request_site(&h, &hosts).is_err());
    }
}
