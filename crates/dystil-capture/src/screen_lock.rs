use std::sync::atomic::{AtomicBool, Ordering};

static SCREEN_IS_LOCKED: AtomicBool = AtomicBool::new(false);

pub fn screen_is_locked() -> bool {
    SCREEN_IS_LOCKED.load(Ordering::SeqCst)
}

pub fn set_screen_locked(locked: bool) {
    SCREEN_IS_LOCKED.store(locked, Ordering::SeqCst);
}
