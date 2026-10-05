//! One cancellation watcher per inference operation, shared by OCR, detection, and cleanup.
use std::{
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::Duration,
};
use tokio_util::sync::CancellationToken;
pub(crate) struct RunGuard {
    done: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}
impl RunGuard {
    pub(crate) fn new(cancel: CancellationToken, options: Arc<ort::session::RunOptions>) -> Self {
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        let signal = done.clone();
        let thread = std::thread::spawn(move || {
            let (lock, cv) = &*signal;
            let mut done = lock.lock().unwrap();
            while !*done {
                if cancel.is_cancelled() {
                    let _ = options.terminate();
                    break;
                }
                done = cv.wait_timeout(done, Duration::from_millis(20)).unwrap().0;
            }
        });
        Self {
            done,
            thread: Some(thread),
        }
    }
}
impl Drop for RunGuard {
    fn drop(&mut self) {
        let (lock, cv) = &*self.done;
        *lock.lock().unwrap() = true;
        cv.notify_all();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
