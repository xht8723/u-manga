//! One cancellable wait immediately before every generation HTTP attempt.
use crate::types::ProviderProfile;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::{sync::Mutex, time::Instant};

type Slots = HashMap<String, Arc<Mutex<Instant>>>;
static SLOTS: OnceLock<parking_lot::Mutex<Slots>> = OnceLock::new();

pub async fn wait(profile: &ProviderProfile) {
    let key = format!("{}|{}|{}", profile.id, profile.service, profile.endpoint);
    let slot = {
        let mut slots = SLOTS.get_or_init(Default::default).lock();
        if slots.len() > 256 {
            slots.retain(|_, slot| {
                Arc::strong_count(slot) > 1
                    || slot.try_lock().map_or(true, |next| {
                        *next + Duration::from_secs(60) > Instant::now()
                    })
            });
        }
        slots
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(Instant::now())))
            .clone()
    };
    // Holding only this service's async guard preserves ordering. Dropping a
    // cancelled caller releases it without reserving a future request slot.
    let mut next = slot.lock().await;
    tokio::time::sleep_until(*next).await;
    let rate = if profile.rate_limit.is_finite() {
        profile.rate_limit.clamp(0.1, 20.)
    } else {
        1.
    };
    *next = Instant::now() + Duration::from_secs_f32(1. / rate);
}
