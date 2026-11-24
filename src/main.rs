//! Menu bar controller for the Logitech Litra Glow.

mod device;
mod mapping;
mod ui;

use objc2::MainThreadMarker;
use std::sync::{Arc, Mutex};

fn main() {
    let mtm = MainThreadMarker::new().expect("must run on the main thread");
    let slot: device::Slot = Arc::new(Mutex::new(None));
    // The worker needs the controller (to wake the main thread), and the
    // controller needs the worker's command channel, so the channel is made
    // first and the worker is spawned once the controller exists.
    let (tx, rx) = std::sync::mpsc::channel();
    let worker_slot = slot.clone();
    ui::run(mtm, slot, tx, move |controller| {
        let handle = ui::MainThreadHandle::new(controller);
        device::spawn(worker_slot, rx, move || handle.notify());
    });
}
