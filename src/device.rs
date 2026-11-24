//! Background worker that owns the HID connection to the light.
//!
//! All HID I/O happens on this thread so a slow or missing device can never
//! freeze the menu. The UI sends [`Cmd`]s; results come back as [`Snapshot`]s
//! through a shared slot plus a wake-up callback.

use crate::mapping::Range;
use litra::{DeviceHandle, DeviceResult, Litra};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread;

/// How long to wait for the light to acknowledge a write.
const ACK_WAIT_MS: i32 = 40;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cmd {
    /// Re-read the state from the light (and reconnect if needed).
    Refresh,
    Power(bool),
    Brightness(u16),
    Temperature(u16),
}

/// Everything the UI needs to know about a connected light.
#[derive(Debug, Clone, PartialEq)]
pub struct Info {
    pub name: String,
    pub on: bool,
    pub lumen: u16,
    pub kelvin: u16,
    pub lumen_range: Range,
    pub kelvin_range: Range,
}

/// `None` means no light is reachable.
pub type Snapshot = Option<Info>;

/// Latest unconsumed snapshot, shared with the UI thread.
pub type Slot = Arc<Mutex<Option<Snapshot>>>;

/// Spawns the worker. `notify` is called (from the worker thread) after a new
/// snapshot has been stored in `slot`.
pub fn spawn(slot: Slot, rx: Receiver<Cmd>, notify: impl Fn() + Send + 'static) {
    thread::Builder::new()
        .name("litra-worker".into())
        .spawn(move || Worker::new(slot, notify).run(rx))
        .expect("failed to spawn worker thread");
}

/// Several queued commands collapsed into the final value of each kind, so a
/// fast slider drag results in one write rather than hundreds.
#[derive(Default, Debug, PartialEq)]
struct Batch {
    refresh: bool,
    power: Option<bool>,
    brightness: Option<u16>,
    temperature: Option<u16>,
}

impl Batch {
    fn push(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Refresh => self.refresh = true,
            Cmd::Power(on) => self.power = Some(on),
            Cmd::Brightness(l) => self.brightness = Some(l),
            Cmd::Temperature(k) => self.temperature = Some(k),
        }
    }
}

struct Worker<N: Fn()> {
    ctx: Option<Litra>,
    handle: Option<DeviceHandle>,
    slot: Slot,
    notify: N,
}

impl<N: Fn()> Worker<N> {
    fn new(slot: Slot, notify: N) -> Self {
        Self {
            ctx: None,
            handle: None,
            slot,
            notify,
        }
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        while let Ok(first) = rx.recv() {
            let mut batch = Batch::default();
            batch.push(first);
            while let Ok(next) = rx.try_recv() {
                batch.push(next);
            }
            self.process(&batch);
        }
    }

    fn process(&mut self, batch: &Batch) {
        // One retry: the cached handle may be stale after an unplug/replug.
        for _ in 0..2 {
            if self.handle.is_none() && !self.connect() {
                break;
            }
            let handle = self.handle.as_ref().expect("connected above");
            match apply(handle, batch) {
                Ok(info) => {
                    if let Some(info) = info {
                        self.publish(Some(info));
                    }
                    return;
                }
                Err(_) => self.handle = None,
            }
        }
        self.publish(None);
    }

    fn connect(&mut self) -> bool {
        if self.ctx.is_none() {
            self.ctx = Litra::new().ok();
        }
        let Some(ctx) = self.ctx.as_mut() else {
            return false;
        };
        if ctx.refresh_connected_devices().is_err() {
            return false;
        }
        let handle = ctx
            .get_connected_devices()
            .find_map(|device| device.open(ctx).ok());
        self.handle = handle;
        self.handle.is_some()
    }

    fn publish(&self, snapshot: Snapshot) {
        *self.slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(snapshot);
        (self.notify)();
    }
}

/// The light answers every write with an acknowledgement packet and also emits
/// notifications when its physical buttons are used. The `litra` crate's
/// getters read the *oldest* queued packet, so unread acks would be mistaken
/// for query responses (yielding 0 lm / 0 K / off). Consume everything pending.
fn drain(h: &DeviceHandle, wait_ms: i32) {
    let mut buf = [0u8; 64];
    // Bounded: a chatty device must not trap the worker here.
    for _ in 0..32 {
        match h.hid_device().read_timeout(&mut buf, wait_ms) {
            Ok(n) if n > 0 => {}
            _ => break,
        }
    }
}

/// Applies the writes in `batch`, then reads the state back if requested.
fn apply(h: &DeviceHandle, batch: &Batch) -> DeviceResult<Option<Info>> {
    // Discard stale packets first so the ack we wait for below is ours.
    drain(h, 0);
    if let Some(on) = batch.power {
        h.set_on(on)?;
        drain(h, ACK_WAIT_MS);
    }
    if let Some(lumen) = batch.brightness {
        h.set_brightness_in_lumen(lumen)?;
        drain(h, ACK_WAIT_MS);
    }
    if let Some(kelvin) = batch.temperature {
        h.set_temperature_in_kelvin(kelvin)?;
        drain(h, ACK_WAIT_MS);
    }
    if !batch.refresh {
        return Ok(None);
    }
    Ok(Some(Info {
        name: h.device_type().to_string(),
        on: h.is_on()?,
        lumen: h.brightness_in_lumen()?,
        kelvin: h.temperature_in_kelvin()?,
        lumen_range: (
            h.minimum_brightness_in_lumen(),
            h.maximum_brightness_in_lumen(),
        ),
        kelvin_range: (
            h.minimum_temperature_in_kelvin(),
            h.maximum_temperature_in_kelvin(),
        ),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_keeps_last_value_of_each_kind() {
        let mut b = Batch::default();
        for c in [
            Cmd::Brightness(30),
            Cmd::Temperature(3000),
            Cmd::Brightness(90),
            Cmd::Power(true),
            Cmd::Power(false),
            Cmd::Refresh,
        ] {
            b.push(c);
        }
        assert_eq!(
            b,
            Batch {
                refresh: true,
                power: Some(false),
                brightness: Some(90),
                temperature: Some(3000)
            }
        );
    }

    /// Needs a real light: `cargo test -- --ignored --nocapture`.
    /// Drives the worker like the UI does and restores the light afterwards.
    #[test]
    #[ignore = "requires a connected Litra light"]
    fn hardware_roundtrip() {
        use std::sync::mpsc::channel;
        use std::time::{Duration, Instant};

        let slot: Slot = Arc::new(Mutex::new(None));
        let (tx, rx) = channel();
        spawn(slot.clone(), rx, || {});

        let read = |tx: &std::sync::mpsc::Sender<Cmd>| -> Info {
            *slot.lock().unwrap() = None;
            tx.send(Cmd::Refresh).unwrap();
            let start = Instant::now();
            loop {
                if let Some(snapshot) = slot.lock().unwrap().take() {
                    return snapshot.expect("light should be connected");
                }
                assert!(start.elapsed() < Duration::from_secs(5), "worker timed out");
                std::thread::sleep(Duration::from_millis(10));
            }
        };

        let original = read(&tx);
        println!("original: {original:?}");

        // Many rapid writes, then a read: the ack packets must not be mistaken
        // for query responses.
        for l in (30..=200).step_by(10) {
            tx.send(Cmd::Brightness(l)).unwrap();
        }
        tx.send(Cmd::Power(true)).unwrap();
        tx.send(Cmd::Brightness(60)).unwrap();
        tx.send(Cmd::Temperature(3000)).unwrap();
        let after = read(&tx);
        println!("after:    {after:?}");
        assert!(after.on);
        assert_eq!(after.lumen, 60);
        assert_eq!(after.kelvin, 3000);

        tx.send(Cmd::Power(false)).unwrap();
        assert!(!read(&tx).on, "power off");
        // A second read must still agree (queue stays clean).
        let again = read(&tx);
        assert_eq!((again.lumen, again.kelvin), (60, 3000));

        tx.send(Cmd::Brightness(original.lumen)).unwrap();
        tx.send(Cmd::Temperature(original.kelvin)).unwrap();
        tx.send(Cmd::Power(original.on)).unwrap();
        assert_eq!(read(&tx), original, "state restored");
    }
}
