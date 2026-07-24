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

use x11::keysym::XK_space;
use x11::xlib::{
    self, Atom, CWBackPixel, CWBorderPixel, CWEventMask, ClientMessage, ConfigureNotify,
    ControlMask, DestroyNotify, Display, ExposureMask, FocusChangeMask, GrabModeAsync, InputHint,
    KeyPress, KeyPressMask, LockMask, Mod1Mask, Mod2Mask, PBaseSize, PMinSize, PSize,
    PropertyChangeMask, ShiftMask, StructureNotifyMask, SubstructureNotifyMask, Window,
    XClientMessageEvent, XConfigureEvent, XEvent, XKeyEvent, XSetWindowAttributes,
};

/// Lock modifiers we must ignore/replicate when grabbing keys so Caps Lock and
/// Num Lock don't defeat the grab. X11 grabs are exact-match on modifiers.
const LOCK_MASKS: [c_uint; 4] = [0, LockMask, Mod2Mask, LockMask | Mod2Mask];

const DEFAULT_WIDTH: c_uint = 800;
const DEFAULT_HEIGHT: c_uint = 600;

/// Close chord for a focused plugin editor (from Motif shortcut settings).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorCloseBinding {
    pub key_name: String,
    pub ctrl_or_cmd: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Default for EditorCloseBinding {
    fn default() -> Self {
        Self {
            key_name: String::from("Q"),
            ctrl_or_cmd: false,
            shift: true,
            alt: false,
        }
    }
}

impl EditorCloseBinding {
    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl_or_cmd {
            parts.push("Ctrl/Cmd".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        parts.push(self.key_name.clone());
        parts.join("+")
    }
}

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
    /// Keycode for Space; passively grabbed while `forward_transport` is on so
    /// Motif transport shortcuts work even when the plugin editor has focus.
    space_keycode: c_uint,
    /// Close-editor chord (passive grab while the editor is focused).
    close_binding: EditorCloseBinding,
    close_keycode: c_uint,
    /// Whether Space is currently grabbed and reported as `TogglePlayback`.
    forward_transport: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorWindowEvent {
    CloseRequested,
    Resized { width: u32, height: u32 },
    /// Space was pressed while the editor had focus and transport forwarding is on.
    TogglePlayback,
}

impl EditorParentWindow {
    pub fn create(
        title: &str,
        host: Option<HostX11>,
        forward_transport: bool,
        close_binding: EditorCloseBinding,
    ) -> Result<Self, String> {
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
            Self::create_on_display(
                display,
                screen,
                title,
                transient_for,
                true,
                forward_transport,
                close_binding,
            )
        }
    }

    unsafe fn create_on_display(
        display: *mut Display,
        screen: c_int,
        title: &str,
        transient_for: Option<u64>,
        owns_display: bool,
        forward_transport: bool,
        close_binding: EditorCloseBinding,
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
            | PropertyChangeMask
            | KeyPressMask;

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

        change_prop8(display, window, net_name, utf8, title.as_bytes());
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

        // Passive key grabs. Close chord is always grabbed. Space (transport)
        // is grabbed only when forwarding is enabled for this plugin; that way a
        // plugin that needs Space in its own UI can opt out. The grab activates
        // only while focus is inside our window (or the embedded plugin child),
        // so it never affects other apps.
        let space_keycode = c_uint::from(xlib::XKeysymToKeycode(display, XK_space as xlib::KeySym));
        let close_keycode = keycode_for_name(display, &close_binding.key_name)?;
        grab_close_binding(display, close_keycode, &close_binding, window);
        if forward_transport {
            grab_key_all_locks(display, space_keycode, 0, window);
        }
        xlib::XFlush(display);

        Ok(Self {
            display,
            window,
            owns_display,
            wm_delete_window: wm_delete,
            space_keycode,
            close_binding,
            close_keycode,
            forward_transport,
        })
    }

    /// Live-update the close chord (e.g. after Settings shortcut remap).
    pub fn set_close_binding(&mut self, close_binding: EditorCloseBinding) -> Result<(), String> {
        if close_binding == self.close_binding {
            return Ok(());
        }
        unsafe {
            ungrab_close_binding(
                self.display,
                self.close_keycode,
                &self.close_binding,
                self.window,
            );
            let close_keycode = keycode_for_name(self.display, &close_binding.key_name)?;
            grab_close_binding(self.display, close_keycode, &close_binding, self.window);
            xlib::XFlush(self.display);
            self.close_binding = close_binding;
            self.close_keycode = close_keycode;
        }
        Ok(())
    }

    /// Enable/disable forwarding Space to Motif transport while the editor is
    /// focused. Toggling adds/removes the passive Space grab live.
    pub fn set_forward_transport(&mut self, forward: bool) {
        if forward == self.forward_transport {
            return;
        }
        self.forward_transport = forward;
        unsafe {
            if forward {
                grab_key_all_locks(self.display, self.space_keycode, 0, self.window);
            } else {
                ungrab_key_all_locks(self.display, self.space_keycode, 0, self.window);
            }
            xlib::XFlush(self.display);
        }
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
            while xlib::XCheckTypedWindowEvent(
                self.display,
                self.window,
                KeyPress,
                event.as_mut_ptr(),
            ) != 0
            {
                let ev = event.assume_init();
                let key: XKeyEvent = ev.key;
                // Ignore Caps Lock / Num Lock so grabs match regardless of lock state.
                let mods = key.state & !(LockMask | Mod2Mask);
                if key.keycode == self.close_keycode
                    && close_binding_matches(&self.close_binding, mods)
                {
                    events.push(EditorWindowEvent::CloseRequested);
                } else if self.forward_transport && key.keycode == self.space_keycode && mods == 0 {
                    events.push(EditorWindowEvent::TogglePlayback);
                }
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

unsafe fn grab_key_all_locks(
    display: *mut Display,
    keycode: c_uint,
    base_mods: c_uint,
    window: Window,
) {
    for extra in LOCK_MASKS {
        xlib::XGrabKey(
            display,
            keycode as c_int,
            base_mods | extra,
            window,
            xlib::False,
            GrabModeAsync,
            GrabModeAsync,
        );
    }
}

unsafe fn ungrab_key_all_locks(
    display: *mut Display,
    keycode: c_uint,
    base_mods: c_uint,
    window: Window,
) {
    for extra in LOCK_MASKS {
        xlib::XUngrabKey(display, keycode as c_int, base_mods | extra, window);
    }
}

fn close_binding_mods(binding: &EditorCloseBinding) -> c_uint {
    let mut mods = 0;
    if binding.ctrl_or_cmd {
        mods |= ControlMask;
    }
    if binding.shift {
        mods |= ShiftMask;
    }
    if binding.alt {
        mods |= Mod1Mask;
    }
    mods
}

fn close_binding_matches(binding: &EditorCloseBinding, mods: c_uint) -> bool {
    let ctrl = (mods & ControlMask) != 0;
    let shift = (mods & ShiftMask) != 0;
    let alt = (mods & Mod1Mask) != 0;
    ctrl == binding.ctrl_or_cmd && shift == binding.shift && alt == binding.alt
}

unsafe fn grab_close_binding(
    display: *mut Display,
    keycode: c_uint,
    binding: &EditorCloseBinding,
    window: Window,
) {
    grab_key_all_locks(display, keycode, close_binding_mods(binding), window);
}

unsafe fn ungrab_close_binding(
    display: *mut Display,
    keycode: c_uint,
    binding: &EditorCloseBinding,
    window: Window,
) {
    ungrab_key_all_locks(display, keycode, close_binding_mods(binding), window);
}

fn keycode_for_name(display: *mut Display, key_name: &str) -> Result<c_uint, String> {
    let keysym = name_to_keysym(key_name)
        .ok_or_else(|| format!("Unsupported close key for plugin editor: {key_name}"))?;
    unsafe {
        let keycode = xlib::XKeysymToKeycode(display, keysym as xlib::KeySym);
        if keycode == 0 {
            return Err(format!("No X keycode for {key_name}"));
        }
        Ok(c_uint::from(keycode))
    }
}

fn name_to_keysym(name: &str) -> Option<u32> {
    use x11::keysym::*;
    Some(match name {
        "Space" => XK_space,
        "Escape" => XK_Escape,
        "Delete" => XK_Delete,
        "Backspace" => XK_BackSpace,
        "Enter" => XK_Return,
        "Tab" => XK_Tab,
        "Insert" => XK_Insert,
        "Home" => XK_Home,
        "End" => XK_End,
        "PageUp" => XK_Page_Up,
        "PageDown" => XK_Page_Down,
        "Left" => XK_Left,
        "Right" => XK_Right,
        "Up" => XK_Up,
        "Down" => XK_Down,
        "A" => XK_a,
        "B" => XK_b,
        "C" => XK_c,
        "D" => XK_d,
        "E" => XK_e,
        "F" => XK_f,
        "G" => XK_g,
        "H" => XK_h,
        "I" => XK_i,
        "J" => XK_j,
        "K" => XK_k,
        "L" => XK_l,
        "M" => XK_m,
        "N" => XK_n,
        "O" => XK_o,
        "P" => XK_p,
        "Q" => XK_q,
        "R" => XK_r,
        "S" => XK_s,
        "T" => XK_t,
        "U" => XK_u,
        "V" => XK_v,
        "W" => XK_w,
        "X" => XK_x,
        "Y" => XK_y,
        "Z" => XK_z,
        "0" => XK_0,
        "1" => XK_1,
        "2" => XK_2,
        "3" => XK_3,
        "4" => XK_4,
        "5" => XK_5,
        "6" => XK_6,
        "7" => XK_7,
        "8" => XK_8,
        "9" => XK_9,
        "F1" => XK_F1,
        "F2" => XK_F2,
        "F3" => XK_F3,
        "F4" => XK_F4,
        "F5" => XK_F5,
        "F6" => XK_F6,
        "F7" => XK_F7,
        "F8" => XK_F8,
        "F9" => XK_F9,
        "F10" => XK_F10,
        "F11" => XK_F11,
        "F12" => XK_F12,
        _ => return None,
    })
}

unsafe fn intern_atom(display: *mut Display, name: &[u8]) -> Atom {
    let c = CString::new(name).unwrap_or_default();
    xlib::XInternAtom(display, c.as_ptr(), xlib::False)
}

unsafe fn change_prop8(
    display: *mut Display,
    window: Window,
    property: Atom,
    typ: Atom,
    data: &[u8],
) {
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
