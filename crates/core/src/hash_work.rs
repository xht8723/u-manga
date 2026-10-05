//! Two cancellable disk hash workers shared by verification and downloads.
use parking_lot::{Condvar, Mutex};
use std::{sync::LazyLock, time::Duration};
use tokio_util::sync::CancellationToken;
static ACTIVE: LazyLock<(Mutex<usize>, Condvar)> =
    LazyLock::new(|| (Mutex::new(0), Condvar::new()));
pub(crate) struct Permit;
pub(crate) fn acquire(cancel: &CancellationToken) -> anyhow::Result<Permit> {
    let (active, changed) = &*ACTIVE;
    let mut count = active.lock();
    loop {
        anyhow::ensure!(!cancel.is_cancelled(), "Cancelled");
        if *count < 2 {
            *count += 1;
            return Ok(Permit);
        }
        changed.wait_for(&mut count, Duration::from_millis(20));
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        let (active, changed) = &*ACTIVE;
        *active.lock() -= 1;
        changed.notify_one();
    }
}
