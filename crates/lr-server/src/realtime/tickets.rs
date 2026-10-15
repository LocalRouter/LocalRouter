//! Process-local, single-use browser credentials and bounded session permits.
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
#[derive(Clone)]
pub struct Ticket {
    pub owner: String,
    pub model: String,
    pub credential_tag: String,
    pub provider_binding: String,
    pub expires_at: i64,
}
#[derive(Default)]
struct Inner {
    tickets: HashMap<String, Ticket>,
    active: HashMap<String, usize>,
}
#[derive(Default)]
pub struct Tickets(Mutex<Inner>);
fn hash(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}
impl Tickets {
    pub fn mint(&self, ticket: Ticket, now: i64) -> Result<String, &'static str> {
        let mut inner = self.0.lock();
        inner.tickets.retain(|_, t| t.expires_at > now);
        if ticket.expires_at <= now || ticket.expires_at > now + 60 {
            return Err("Local realtime ticket opening TTL is at most 60 seconds");
        }
        if inner.tickets.len() >= 512
            || inner
                .tickets
                .values()
                .filter(|t| t.owner == ticket.owner)
                .count()
                >= 16
        {
            return Err("Too many pending realtime tickets");
        }
        let value = format!("ek_lr_{}", uuid::Uuid::new_v4().simple());
        inner.tickets.insert(hash(&value), ticket);
        Ok(value)
    }
    pub fn take(&self, value: &str, now: i64) -> Option<Ticket> {
        if !value.starts_with("ek_lr_") || value.len() > 80 {
            return None;
        }
        let mut inner = self.0.lock();
        inner.tickets.retain(|_, t| t.expires_at > now);
        inner.tickets.remove(&hash(value))
    }
    pub fn reserve(self: &Arc<Self>, owner: &str) -> Result<SessionPermit, &'static str> {
        let mut inner = self.0.lock();
        if inner.active.values().sum::<usize>() >= 16
            || inner.active.get(owner).copied().unwrap_or(0) >= 2
        {
            return Err("Realtime session concurrency limit reached");
        }
        *inner.active.entry(owner.into()).or_default() += 1;
        Ok(SessionPermit {
            tickets: self.clone(),
            owner: owner.into(),
        })
    }
}
pub struct SessionPermit {
    tickets: Arc<Tickets>,
    owner: String,
}
impl Drop for SessionPermit {
    fn drop(&mut self) {
        let mut inner = self.tickets.0.lock();
        if let Some(count) = inner.active.get_mut(&self.owner) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                inner.active.remove(&self.owner);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ticket(now: i64) -> Ticket {
        Ticket {
            owner: "owner".into(),
            model: "gateway/native".into(),
            credential_tag: "tag".into(),
            provider_binding: "binding".into(),
            expires_at: now + 60,
        }
    }
    #[test]
    fn browser_tickets_are_atomic_single_use_expiring_and_bounded() {
        let registry = Arc::new(Tickets::default());
        let value = registry.mint(ticket(100), 100).unwrap();
        assert!(!registry.0.lock().tickets.contains_key(&value));
        let mut workers = Vec::new();
        for _ in 0..8 {
            let registry = registry.clone();
            let value = value.clone();
            workers.push(std::thread::spawn(move || {
                registry.take(&value, 101).is_some()
            }));
        }
        assert_eq!(
            workers
                .into_iter()
                .filter_map(|t| t.join().ok())
                .filter(|v| *v)
                .count(),
            1
        );
        let value = registry.mint(ticket(100), 100).unwrap();
        assert!(registry.take(&value, 160).is_none());
        for _ in 0..16 {
            registry.mint(ticket(200), 200).unwrap();
        }
        assert!(registry.mint(ticket(200), 200).is_err());
    }
    #[test]
    fn dropping_connections_releases_owner_and_global_concurrency() {
        let registry = Arc::new(Tickets::default());
        let one = registry.reserve("one").unwrap();
        let two = registry.reserve("one").unwrap();
        assert!(registry.reserve("one").is_err());
        drop(two);
        registry.reserve("one").unwrap();
        drop(one);
        let guards: Vec<_> = (0..16)
            .map(|i| registry.reserve(&format!("owner-{i}")).unwrap())
            .collect();
        assert!(registry.reserve("extra").is_err());
        drop(guards);
        assert!(registry.reserve("extra").is_ok());
    }
}
