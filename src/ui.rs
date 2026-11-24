//! The menu bar item and its menu (native AppKit, built with objc2).

use crate::device::{Cmd, Info, Slot, Snapshot};
use crate::mapping::{self, Range};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{DefinedClass, Message};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSColor, NSControlStateValueOff,
    NSControlStateValueOn, NSFont, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSSlider,
    NSStatusBar, NSStatusItem, NSSwitch, NSTextAlignment, NSTextField, NSVariableStatusItemLength,
    NSView,
};
use objc2_foundation::{
    NSBundle, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoopCommonModes, NSSize, NSString,
};
use objc2_service_management::{SMAppService, SMAppServiceStatus};
use std::cell::{Cell, OnceCell};
use std::sync::mpsc::Sender;

const MENU_WIDTH: f64 = 250.0;
const PAD: f64 = 14.0;

// Defaults used until the light reports its real ranges (Litra Glow).
const DEFAULT_LUMEN: Range = (20, 250);
const DEFAULT_KELVIN: Range = (2700, 6500);

struct Ui {
    status_item: Retained<NSStatusItem>,
    header: Retained<NSMenuItem>,
    power: Retained<NSSwitch>,
    brightness: Retained<NSSlider>,
    brightness_value: Retained<NSTextField>,
    warmth: Retained<NSSlider>,
    warmth_value: Retained<NSTextField>,
    login: Retained<NSMenuItem>,
}

pub struct Ivars {
    tx: Sender<Cmd>,
    slot: Slot,
    ui: OnceCell<Ui>,
    lumen_range: Cell<Range>,
    kelvin_range: Cell<Range>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "LGController"]
    #[ivars = Ivars]
    pub struct Controller;

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSMenuDelegate for Controller {
        /// Re-read the light every time the menu opens, so changes made with
        /// its physical buttons (or a hot-plug) are reflected.
        #[unsafe(method(menuWillOpen:))]
        fn menu_will_open(&self, _menu: &NSMenu) {
            let _ = self.ivars().tx.send(Cmd::Refresh);
            self.sync_login_item();
        }
    }

    impl Controller {
        #[unsafe(method(togglePower:))]
        fn toggle_power(&self, sender: &NSSwitch) {
            let on = sender.state() == NSControlStateValueOn;
            let _ = self.ivars().tx.send(Cmd::Power(on));
            self.update_icon(Some(on));
        }

        #[unsafe(method(brightnessChanged:))]
        fn brightness_changed(&self, sender: &NSSlider) {
            self.wake_if_off();
            let fraction = sender.doubleValue();
            let lumen = mapping::lumen_from_fraction(self.ivars().lumen_range.get(), fraction);
            self.ui().brightness_value.setStringValue(&percent_label(fraction));
            let _ = self.ivars().tx.send(Cmd::Brightness(lumen));
        }

        #[unsafe(method(warmthChanged:))]
        fn warmth_changed(&self, sender: &NSSlider) {
            self.wake_if_off();
            let kelvin = mapping::kelvin_from_warmth(self.ivars().kelvin_range.get(), sender.doubleValue());
            self.ui().warmth_value.setStringValue(&kelvin_label(kelvin));
            let _ = self.ivars().tx.send(Cmd::Temperature(kelvin));
        }

        #[unsafe(method(toggleLogin:))]
        fn toggle_login(&self, _sender: &NSMenuItem) {
            let service = unsafe { SMAppService::mainAppService() };
            let enabled = unsafe { service.status() } == SMAppServiceStatus::Enabled;
            let result = unsafe {
                if enabled { service.unregisterAndReturnError() } else { service.registerAndReturnError() }
            };
            if let Err(error) = result {
                eprintln!("launch at login: {}", error.localizedDescription());
            }
            self.sync_login_item();
        }

        /// Called on the main thread by the worker when a snapshot is ready.
        #[unsafe(method(snapshotReady:))]
        fn snapshot_ready(&self, _object: Option<&AnyObject>) {
            let pending = self.ivars().slot.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(snapshot) = pending {
                self.apply(snapshot);
            }
        }
    }
);

impl Controller {
    fn ui(&self) -> &Ui {
        self.ivars()
            .ui
            .get()
            .expect("UI is built before any action can fire")
    }

    /// Reflects the current login-item registration in the menu. Only a
    /// bundled `.app` can register itself, so the item is disabled otherwise.
    fn sync_login_item(&self) {
        let login = &self.ui().login;
        let bundled = NSBundle::mainBundle()
            .bundlePath()
            .to_string()
            .ends_with(".app");
        login.setEnabled(bundled);
        let enabled = bundled
            && unsafe { SMAppService::mainAppService().status() } == SMAppServiceStatus::Enabled;
        login.setState(if enabled {
            NSControlStateValueOn
        } else {
            NSControlStateValueOff
        });
    }

    /// Moving a slider while the light is off is a clear intent to see light.
    fn wake_if_off(&self) {
        let ui = self.ui();
        if ui.power.state() != NSControlStateValueOn {
            ui.power.setState(NSControlStateValueOn);
            let _ = self.ivars().tx.send(Cmd::Power(true));
            self.update_icon(Some(true));
        }
    }

    /// `Some(on)` for a connected light, `None` for disconnected.
    fn update_icon(&self, state: Option<bool>) {
        let (symbol, description) = match state {
            Some(true) => ("lightbulb.fill", "Light on"),
            Some(false) => ("lightbulb", "Light off"),
            None => ("lightbulb.slash", "Light not connected"),
        };
        let Some(button) = self.ui().status_item.button(self.mtm()) else {
            return;
        };
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol),
            Some(&NSString::from_str(description)),
        );
        if let Some(image) = &image {
            image.setTemplate(true);
        }
        button.setImage(image.as_deref());
    }

    fn apply(&self, snapshot: Snapshot) {
        let ui = self.ui();
        match snapshot {
            Some(Info {
                name,
                on,
                lumen,
                kelvin,
                lumen_range,
                kelvin_range,
            }) => {
                self.ivars().lumen_range.set(lumen_range);
                self.ivars().kelvin_range.set(kelvin_range);
                ui.header.setTitle(&NSString::from_str(&name));
                ui.power.setState(if on {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
                let brightness = mapping::fraction_from_lumen(lumen_range, lumen);
                ui.brightness.setDoubleValue(brightness);
                ui.brightness_value
                    .setStringValue(&percent_label(brightness));
                let warmth = mapping::warmth_from_kelvin(kelvin_range, kelvin);
                ui.warmth.setDoubleValue(warmth);
                ui.warmth_value.setStringValue(&kelvin_label(kelvin));
                set_enabled(ui, true);
                self.update_icon(Some(on));
            }
            None => {
                ui.header
                    .setTitle(&NSString::from_str("No Litra light found"));
                set_enabled(ui, false);
                self.update_icon(None);
            }
        }
    }
}

fn set_enabled(ui: &Ui, enabled: bool) {
    ui.power.setEnabled(enabled);
    ui.brightness.setEnabled(enabled);
    ui.warmth.setEnabled(enabled);
}

fn percent_label(fraction: f64) -> Retained<NSString> {
    NSString::from_str(&format!("{}%", (fraction * 100.0).round() as u32))
}

fn kelvin_label(kelvin: u16) -> Retained<NSString> {
    NSString::from_str(&format!("{kelvin} K"))
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}

fn label(
    mtm: MainThreadMarker,
    text: &str,
    frame: NSRect,
    secondary: bool,
    right: bool,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    field.setFrame(frame);
    field.setFont(Some(&NSFont::menuFontOfSize(0.0)));
    if secondary {
        field.setTextColor(Some(&NSColor::secondaryLabelColor()));
    }
    if right {
        field.setAlignment(NSTextAlignment::Right);
    }
    field
}

/// A menu row: a title on the left, a live value on the right, a slider below.
fn slider_row(
    mtm: MainThreadMarker,
    target: &Controller,
    title: &str,
    action: objc2::runtime::Sel,
) -> (
    Retained<NSMenuItem>,
    Retained<NSSlider>,
    Retained<NSTextField>,
) {
    let height = 50.0;
    let view = NSView::initWithFrame(mtm.alloc(), rect(0.0, 0.0, MENU_WIDTH, height));
    let inner = MENU_WIDTH - 2.0 * PAD;

    let title = label(
        mtm,
        title,
        rect(PAD, height - 22.0, inner / 2.0, 17.0),
        false,
        false,
    );
    let value = label(
        mtm,
        "",
        rect(PAD + inner / 2.0, height - 22.0, inner / 2.0, 17.0),
        true,
        true,
    );

    let slider = unsafe {
        NSSlider::sliderWithValue_minValue_maxValue_target_action(
            0.5,
            0.0,
            1.0,
            Some(target.as_ref()),
            Some(action),
            mtm,
        )
    };
    slider.setFrame(rect(PAD, 6.0, inner, 20.0));

    view.addSubview(&title);
    view.addSubview(&value);
    view.addSubview(&slider);
    let item = NSMenuItem::new(mtm);
    item.setView(Some(&view));
    (item, slider, value)
}

fn power_row(
    mtm: MainThreadMarker,
    target: &Controller,
) -> (Retained<NSMenuItem>, Retained<NSSwitch>) {
    let height = 34.0;
    let view = NSView::initWithFrame(mtm.alloc(), rect(0.0, 0.0, MENU_WIDTH, height));
    let title = label(
        mtm,
        "Power",
        rect(PAD, (height - 17.0) / 2.0, 120.0, 17.0),
        false,
        false,
    );

    let switch = NSSwitch::new(mtm);
    let size = switch.fittingSize();
    switch.setFrame(rect(
        MENU_WIDTH - PAD - size.width,
        (height - size.height) / 2.0,
        size.width,
        size.height,
    ));
    unsafe {
        switch.setTarget(Some(target.as_ref()));
        switch.setAction(Some(sel!(togglePower:)));
    }

    view.addSubview(&title);
    view.addSubview(&switch);
    let item = NSMenuItem::new(mtm);
    item.setView(Some(&view));
    (item, switch)
}

/// Builds the status item and menu, then runs the app until quit.
pub fn run(
    mtm: MainThreadMarker,
    slot: Slot,
    tx: Sender<Cmd>,
    spawn_worker: impl FnOnce(&Controller),
) {
    let app = NSApplication::sharedApplication(mtm);
    // Menu bar only: no Dock icon, no app menu.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let this = mtm.alloc::<Controller>().set_ivars(Ivars {
        tx,
        slot,
        ui: OnceCell::new(),
        lumen_range: Cell::new(DEFAULT_LUMEN),
        kelvin_range: Cell::new(DEFAULT_KELVIN),
    });
    let controller: Retained<Controller> = unsafe { msg_send![super(this), init] };

    let menu = NSMenu::new(mtm);
    menu.setAutoenablesItems(false);
    menu.setDelegate(Some(ProtocolObject::from_ref(&*controller)));

    let header = NSMenuItem::new(mtm);
    header.setTitle(&NSString::from_str("Looking for light…"));
    header.setEnabled(false);
    menu.addItem(&header);
    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let (power_item, power) = power_row(mtm, &controller);
    menu.addItem(&power_item);
    let (brightness_item, brightness, brightness_value) =
        slider_row(mtm, &controller, "Brightness", sel!(brightnessChanged:));
    menu.addItem(&brightness_item);
    let (warmth_item, warmth, warmth_value) =
        slider_row(mtm, &controller, "Warmth", sel!(warmthChanged:));
    menu.addItem(&warmth_item);

    menu.addItem(&NSMenuItem::separatorItem(mtm));
    let login = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str("Launch at Login"),
            Some(sel!(toggleLogin:)),
            &NSString::new(),
        )
    };
    unsafe { login.setTarget(Some(controller.as_ref())) };
    menu.addItem(&login);
    let quit = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str("Quit Litra Glow"),
            Some(sel!(terminate:)),
            &NSString::from_str("q"),
        )
    };
    menu.addItem(&quit);

    let status_item =
        NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    status_item.setMenu(Some(&menu));

    let ui = Ui {
        status_item,
        header,
        power,
        brightness,
        brightness_value,
        warmth,
        warmth_value,
        login,
    };
    set_enabled(&ui, false);
    controller.ivars().ui.set(ui).ok().expect("built once");
    controller.update_icon(None);
    controller.sync_login_item();

    spawn_worker(&controller);
    let _ = controller.ivars().tx.send(Cmd::Refresh);
    app.run();
}

/// Wrapper to move a retained controller pointer into the worker thread. The
/// only thing the worker ever does with it is `performSelectorOnMainThread`,
/// which is explicitly designed to be called from any thread.
pub struct MainThreadHandle(Retained<AnyObject>);
unsafe impl Send for MainThreadHandle {}

impl MainThreadHandle {
    pub fn new(controller: &Controller) -> Self {
        let object: &AnyObject = controller.as_ref();
        Self(object.retain())
    }

    /// Asks the main thread to consume the pending snapshot. Uses the common
    /// run-loop modes so updates also land while the menu is open (tracking).
    pub fn notify(&self) {
        unsafe {
            let modes = objc2_foundation::NSArray::from_slice(&[NSRunLoopCommonModes]);
            let nil: Option<&AnyObject> = None;
            let _: () = msg_send![
                &*self.0,
                performSelectorOnMainThread: sel!(snapshotReady:),
                withObject: nil,
                waitUntilDone: false,
                modes: &*modes
            ];
        }
    }
}
