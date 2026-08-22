// macos_open.rs
//
// Catches the file(s) macOS opened this app with (Finder "Open With", or a
// file opened while already running) via AppKit's `application:openURLs:`
// delegate method. We can't install our own NSApplicationDelegate (winit
// 0.30.13 already sets its own and crashes if replaced -- see
// https://github.com/rust-windowing/winit/issues/4458), so instead we use
// the Objective-C runtime to add `application:openURLs:` directly onto
// winit's existing delegate class at runtime ("method swizzling"), via raw
// C FFI against libobjc.
//
// Ported from https://github.com/Barni228/slint_mac_open_with, adapted so
// `report_paths` forwards a single path at a time (matching this app's
// one-file-at-a-time `analyze_file` flow) instead of joining multiple
// selected files into one comma-separated display string.

#![cfg(target_os = "macos")]

use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender, channel};

static FILE_OPENED_SENDER: OnceLock<Sender<String>> = OnceLock::new();

// If Finder passes multiple files at once (multi-select -> Open With), only
// the first is forwarded -- the UI here shows one file's info at a time.
fn report_paths(paths: Vec<String>) {
    if let Some(first) = paths.into_iter().next()
        && let Some(tx) = FILE_OPENED_SENDER.get()
    {
        let _ = tx.send(first);
    }
}

/// Sets up the result channel. Call once, before anything else.
pub fn take_receiver() -> Receiver<String> {
    let (tx, rx) = channel();
    FILE_OPENED_SENDER
        .set(tx)
        .expect("take_receiver must only be called once");
    rx
}

type Id = *mut c_void;
type Sel = *mut c_void;
type Class = *mut c_void;
type Boolean = u8;
// The generic function-pointer type expected by class_addMethod. Real IMPs
// are cast to/from this via transmute at the call site.
type Imp = unsafe extern "C" fn();

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn object_getClass(obj: Id) -> Class;
    fn class_addMethod(cls: Class, name: Sel, imp: Imp, types: *const c_char) -> Boolean;

    // objc_msgSend is polymorphic in its real C signature; we transmute this
    // declaration to the specific signature needed at each call site below.
    fn objc_msgSend();
}

unsafe fn sel(name: &str) -> Sel {
    unsafe { sel_registerName(CString::new(name).unwrap().as_ptr()) }
}

// objc_msgSend's real signature depends on the call site; these two generic
// wrappers cover every shape used below (no args, or one u64 arg) by
// transmuting to `unsafe extern "C" fn(..) -> R` for whichever `R` is
// inferred at the call site.
unsafe fn msg_send0<R>(receiver: Id, selector: Sel) -> R {
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel) -> R = std::mem::transmute(objc_msgSend as *const ());
        f(receiver, selector)
    }
}

unsafe fn msg_send1<R>(receiver: Id, selector: Sel, arg: u64) -> R {
    unsafe {
        let f: unsafe extern "C" fn(Id, Sel, u64) -> R =
            std::mem::transmute(objc_msgSend as *const ());
        f(receiver, selector, arg)
    }
}

/// Adds an `application:openURLs:` method to whatever class `NSApp`'s
/// *current* delegate is (installed by winit itself), without changing
/// which object is the delegate. Call this after `AppWindow::new()` (i.e.
/// after winit has created NSApplication and set its own delegate).
pub fn install_open_urls_swizzle() {
    unsafe {
        let ns_application_class_name = CString::new("NSApplication").unwrap();
        let ns_app_class = objc_getClass(ns_application_class_name.as_ptr());
        if ns_app_class.is_null() {
            return;
        }

        let shared_app: Id = msg_send0(ns_app_class, sel("sharedApplication"));
        if shared_app.is_null() {
            return;
        }

        let delegate: Id = msg_send0(shared_app, sel("delegate"));
        if delegate.is_null() {
            return;
        }

        let delegate_class = object_getClass(delegate);
        if delegate_class.is_null() {
            return;
        }

        let selector = sel("application:openURLs:");
        // "v@:@@" = void return; self, _cmd, and two object args.
        let types = CString::new("v@:@@").unwrap();
        let imp: Imp = std::mem::transmute(application_open_urls as *const ());
        class_addMethod(delegate_class, selector, imp, types.as_ptr());
        // The runtime keeps the type-encoding pointer rather than copying
        // the string, so it has to outlive the class -- i.e. the process.
        // Leaking these few bytes once is the simplest way to guarantee it.
        std::mem::forget(types);
    }
}

/// The IMP for `application:openURLs:`, matching the ObjC method signature
/// `- (void)application:(NSApplication *)application openURLs:(NSArray<NSURL *> *)urls;`
/// i.e. C signature `void fn(id self, SEL _cmd, id application, id urls)`.
///
/// Wrapped in catch_unwind: this is called directly by AppKit across the ObjC
/// ABI boundary, and Rust aborts the whole process if a panic unwinds past an
/// `extern "C" fn` -- better to drop this one event than take the app down
/// over a single unexpected URL.
extern "C" fn application_open_urls(_this: Id, _cmd: Sel, _application: Id, urls: Id) {
    let _ = std::panic::catch_unwind(|| unsafe {
        if urls.is_null() {
            return;
        }

        let count: u64 = msg_send0(urls, sel("count"));

        let mut paths = Vec::new();
        for i in 0..count {
            let url: Id = msg_send1(urls, sel("objectAtIndex:"), i);
            if url.is_null() {
                continue;
            }
            // NSURL#path hands back the decoded filesystem path directly,
            // so there's no need to parse "file://...%20..." ourselves.
            let path: Id = msg_send0(url, sel("path"));
            let ptr: *const c_char = msg_send0(path, sel("UTF8String"));
            if ptr.is_null() {
                continue;
            }
            let path = CStr::from_ptr(ptr).to_string_lossy().into_owned();
            paths.push(path);
        }

        report_paths(paths);
    });
}
