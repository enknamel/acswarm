//! Logging characters off on the way out.
//!
//! Closing the client used to send a bare disconnect -- and closing the
//! window sent it only for the session on screen, so every other
//! character being played was simply dropped. A disconnect is the
//! connection going away, which the server ends as an abrupt stop. The
//! game's own logout is a message, and the server acts on it when its
//! world thread gets to it: after a disconnect sent in the same moment has
//! already been acted on. So characters are logged off first, and
//! disconnected once the server says each is gone or a short wait is up.

use std::time::{Duration, Instant};

use crate::Client;

/// How long to wait for the server to finish logging characters off
/// before disconnecting anyway. Its logout saves and plays an animation;
/// a few seconds is plenty, and a client that will not close is worse
/// than one that closes a moment early.
pub const LOG_OFF_WAIT: Duration = Duration::from_secs(3);

/// Log every character off, keep their sessions ticking until the server
/// has finished with each or `wait` is up, then disconnect them all. For
/// the ways out that have no frame loop to wait in.
pub fn log_off_all(clients: &mut [&mut Client], wait: Duration) {
    let start = Instant::now();
    for c in clients.iter_mut() {
        c.log_off(start);
    }
    let mut last = start;
    while start.elapsed() < wait && !clients.iter().all(|c| c.logged_off()) {
        std::thread::sleep(Duration::from_millis(50));
        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;
        for c in clients.iter_mut() {
            let _ = c.tick(None, dt, now);
            let _ = c.drain_events();
        }
    }
    let now = Instant::now();
    for c in clients.iter_mut() {
        c.disconnect(now);
    }
}
