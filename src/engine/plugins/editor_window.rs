//! Native X11 parent window for embedding CLAP/VST3 editors (Linux only).
//!
//! The parent lives on a dedicated Xlib `Display` connection, never winit's.
//! winit drives its own connection through XCB (`XGetXCBConnection`); mixing
//! Xlib event-queue calls (`XCheckTypedWindowEvent`) with XCB event reading on
//! the same connection loses our `WM_DELETE_WINDOW`/`ConfigureNotify` events, so
//! Hyprland's close request silently does nothing. Owning a separate connection
//! lets Motif fully drain its own queue. Plugin input is unaffected: the plugin
//! opens its own X connection to the parent window id, independent of which
//! connection created the window.

#![cfg(target_os = "linux")]

use std::ffi::CString;
use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_long, c_uint};
use std::ptr;

use x11::xlib::{
    self, Atom, ClientMessage, ConfigureNotify, DestroyNotify, Display, ExposureMask,
    FocusChangeMask, InputHint, PBaseSize, PMinSize, PSize, PropertyChangeMask,
    StructureNotifyMask, SubstructureNotifyMask, Window, XClientMessageEvent, XConfigureEvent,
    XEvent, XSetWindowAttributes, CWBackPixel, CWBorderPixel, CWEventMask,
};

const DEFAULT_WIDTH: c_uint = 800;
const DEFAULT_HEIGHT: c_uint = 600;

/// X11 hints from Motif's own window (from eframe / winit). The editor parent
/// runs on its own connection, so we only carry the screen index and the
/// `transient_for` target (Motif's main window id) for WM association.
#[derive(Clone, Copy)]
pub struct HostX11 {
    pub screen: i32,
    pub transient_for: Option<u64>,
}

/// Host-owned top-level X11 window that a plugin editor attaches into.
pub struct EditorParentWindow {
    display: *mut Display,
    window: Window,
    /// When true we opened this Display ourselves and must XCloseDisplay on drop.
    owns_display: bool,
    wm_delete_window: Atom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorWindowEvent {
    CloseRequested,
    Resized { width: u32, height: u32 },
}

impl EditorParentWindow {
    pub fn create(title: &str, host: Option<HostX11>) -> Result<Self, String> {
        unsafe {
            // Always a dedicated connection (see module docs). `host` is used only
            // for `transient_for` (so Hyprland associates/floats the dialog against
            // Motif's main window) and the screen index; both are server-global
            // under the same X server / XWayland instance.
            let display = xlib::XOpenDisplay(ptr::null());
            if display.is_null() {
                return Err(String::from(
                    "XOpenDisplay failed. Plugin editors need X11 or XWayland.",
                ));
            }
            let screen = host
                .map(|h| h.screen)
                .unwrap_or_else(|| xlib::XDefaultScreen(display));
            let transient_for = host.and_then(|h| h.transient_for);
            Self::create_on_display(display, screen, title, transient_for, true)
        }
    }

    unsafe fn create_on_display(
        display: *mut Display,
        screen: c_int,
        title: &str,
        transient_for: Option<u64>,
        owns_display: bool,
    ) -> Result<Self, String> {
        let root = xlib::XRootWindow(display, screen);
        let black = xlib::XBlackPixel(display, screen);

        let mut attrs: XSetWindowAttributes = MaybeUninit::zeroed().assume_init();
        attrs.background_pixel = black;
        attrs.border_pixel = black;
        attrs.event_mask = StructureNotifyMask
            | ExposureMask
            | FocusChangeMask
            | SubstructureNotifyMask
            | PropertyChangeMask;

        let window = xlib::XCreateWindow(
            display,
            root,
            80,
            80,
            DEFAULT_WIDTH,
            DEFAULT_HEIGHT,
            0,
            xlib::CopyFromParent,
            xlib::InputOutput as c_uint,
            ptr::null_mut(),
            CWBackPixel | CWBorderPixel | CWEventMask,
            &mut attrs,
        );
        if window == 0 {
            if owns_display {
                xlib::XCloseDisplay(display);
            }
            return Err(String::from("XCreateWindow failed"));
        }

        let title_c = CString::new(title).unwrap_or_else(|_| CString::new("Plugin").unwrap());
        xlib::XStoreName(display, window, title_c.as_ptr());

        // WM_CLASS = motif-plugin-editor / MotifPluginEditor
        let class_bytes = b"motif-plugin-editor\0MotifPluginEditor\0";
        let class_hint = xlib::XAllocClassHint();
        if !class_hint.is_null() {
            // XClassHint expects *mut c_char; leak stable CStrings for the call.
            let instance = CString::new("motif-plugin-editor").unwrap();
            let class = CString::new("MotifPluginEditor").unwrap();
            (*class_hint).res_name = instance.as_ptr() as *mut _;
            (*class_hint).res_class = class.as_ptr() as *mut _;
            xlib::XSetClassHint(display, window, class_hint);
            xlib::XFree(class_hint.cast());
            // CStrings dropped after SetClassHint copies into server props — XSetClassHint
            // reads the strings during the call, so keep them alive until then (done).
            let _ = (instance, class);
        }

        // Dialog type so Hyprland floats the window.
        let utf8 = intern_atom(display, b"UTF8_STRING");
        let net_name = intern_atom(display, b"_NET_WM_NAME");
        let net_type = intern_atom(display, b"_NET_WM_WINDOW_TYPE");
        let type_dialog = intern_atom(display, b"_NET_WM_WINDOW_TYPE_DIALOG");
        let type_utility = intern_atom(display, b"_NET_WM_WINDOW_TYPE_UTILITY");
        let wm_protocols = intern_atom(display, b"WM_PROTOCOLS");
        let wm_delete = intern_atom(display, b"WM_DELETE_WINDOW");

        change_prop8(
            display,
            window,
            net_name,
            utf8,
            title.as_bytes(),
        );
        let types = [type_dialog, type_utility];
        change_prop_atom(display, window, net_type, &types);

        let mut protocols = [wm_delete];
        xlib::XSetWMProtocols(display, window, protocols.as_mut_ptr(), 1);

        // InputHint: tell the WM this window takes keyboard/pointer focus.
        let hints = xlib::XAllocWMHints();
        if !hints.is_null() {
            (*hints).flags = InputHint as c_long;
            (*hints).input = 1; // True
            xlib::XSetWMHints(display, window, hints);
            xlib::XFree(hints.cast());
        }

        if let Some(parent) = transient_for {
            let parent_w = parent as Window;
            xlib::XSetTransientForHint(display, window, parent_w);
        }

        set_size_hints(display, window, DEFAULT_WIDTH, DEFAULT_HEIGHT);

        xlib::XMapRaised(display, window);
        // Do not XSetInputFocus here — fighting Hyprland/XWayland breaks plugin clicks.
        xlib::XFlush(display);

        let _ = (wm_protocols, class_bytes);

        Ok(Self {
            display,
            window,
            owns_display,
            wm_delete_window: wm_delete,
        })
    }

    pub fn x11_window_id(&self) -> u64 {
        self.window as u64
    }

    pub fn resize(&self, width: u32, height: u32) -> Result<(), String> {
        let width = width.clamp(64, 8192) as c_uint;
        let height = height.clamp(64, 8192) as c_uint;
        unsafe {
            set_size_hints(self.display, self.window, width, height);
            xlib::XResizeWindow(self.display, self.window, width, height);
            xlib::XFlush(self.display);
        }
        Ok(())
    }

    /// Drain events for *this* window only (safe on Motif's shared Display).
    pub fn poll_events(&self) -> Result<Vec<EditorWindowEvent>, String> {
        let mut events = Vec::new();
        unsafe {
            let mut event = MaybeUninit::<XEvent>::uninit();
            // ClientMessage (close), ConfigureNotify, DestroyNotify
            while xlib::XCheckTypedWindowEvent(
                self.display,
                self.window,
                ClientMessage,
                event.as_mut_ptr(),
            ) != 0
            {
                let ev = event.assume_init();
                let msg: XClientMessageEvent = ev.client_message;
                if msg.message_type == intern_atom(self.display, b"WM_PROTOCOLS")
                    && msg.data.get_long(0) == self.wm_delete_window as c_long
                {
                    events.push(EditorWindowEvent::CloseRequested);
                }
            }
            while xlib::XCheckTypedWindowEvent(
                self.display,
                self.window,
                ConfigureNotify,
                event.as_mut_ptr(),
            ) != 0
            {
                let ev = event.assume_init();
                let cfg: XConfigureEvent = ev.configure;
                if cfg.window == self.window {
                    events.push(EditorWindowEvent::Resized {
                        width: cfg.width as u32,
                        height: cfg.height as u32,
                    });
                }
            }
            while xlib::XCheckTypedWindowEvent(
                self.display,
                self.window,
                DestroyNotify,
                event.as_mut_ptr(),
            ) != 0
            {
                events.push(EditorWindowEvent::CloseRequested);
            }
            xlib::XFlush(self.display);
        }
        Ok(events)
    }
}

impl Drop for EditorParentWindow {
    fn drop(&mut self) {
        unsafe {
            if !self.display.is_null() && self.window != 0 {
                xlib::XDestroyWindow(self.display, self.window);
                xlib::XFlush(self.display);
            }
            if self.owns_display && !self.display.is_null() {
                xlib::XCloseDisplay(self.display);
            }
        }
    }
}

/// Must run before any plugin (JUCE/Vital) opens an Xlib Display in-process.
pub fn init_xlib_threads() {
    unsafe {
        let _ = xlib::XInitThreads();
    }
}

unsafe fn intern_atom(display: *mut Display, name: &[u8]) -> Atom {
    let c = CString::new(name).unwrap_or_default();
    xlib::XInternAtom(display, c.as_ptr(), xlib::False)
}

unsafe fn change_prop8(display: *mut Display, window: Window, property: Atom, typ: Atom, data: &[u8]) {
    xlib::XChangeProperty(
        display,
        window,
        property,
        typ,
        8,
        xlib::PropModeReplace,
        data.as_ptr(),
        data.len() as c_int,
    );
}

unsafe fn change_prop_atom(display: *mut Display, window: Window, property: Atom, atoms: &[Atom]) {
    xlib::XChangeProperty(
        display,
        window,
        property,
        xlib::XA_ATOM,
        32,
        xlib::PropModeReplace,
        atoms.as_ptr().cast(),
        atoms.len() as c_int,
    );
}

unsafe fn set_size_hints(display: *mut Display, window: Window, width: c_uint, height: c_uint) {
    let hints = xlib::XAllocSizeHints();
    if hints.is_null() {
        return;
    }
    (*hints).flags = (PSize | PMinSize | PBaseSize) as c_long;
    (*hints).width = width as c_int;
    (*hints).height = height as c_int;
    (*hints).min_width = width as c_int;
    (*hints).min_height = height as c_int;
    (*hints).base_width = width as c_int;
    (*hints).base_height = height as c_int;
    xlib::XSetWMNormalHints(display, window, hints);
    xlib::XFree(hints.cast());
}
