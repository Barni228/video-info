// macos_open.rs
//
// Catches the file(s) macOS opened this app with -- both a fresh Finder
// "Open With" launch and a file opened while the app is already running --
// via a single mechanism: AppKit's `application:openURLs:` delegate method.
// That method is Apple's documented replacement for the old odoc/openFile(s)
// Apple Events and fires for both cases; there's no need for a second,
// separate handler for the "already running" case.
//
// The obvious way to receive it -- installing our own NSApplicationDelegate
// via `app.setDelegate(...)` -- crashes, because winit 0.30.13 (what Slint
// currently pulls in) registers its own internal delegate and swizzles
// NSApplication's `sendEvent:` in a way that assumes its own delegate object
// is still in place; replacing it panics deep in winit's event dispatch on
// the very next event (confirmed, currently-open upstream bug:
// https://github.com/rust-windowing/winit/issues/4458).
//
// So instead of replacing the delegate object, we leave winit's own delegate
// object in place (satisfying whatever internal checks it does) and use the
// Objective-C runtime to add an `application:openURLs:` method directly onto
// *its* class at runtime ("method swizzling" / dynamic patching -- a
// standard, decades-old ObjC technique for exactly this "I need to teach an
// existing delegate a new trick" situation). `class_addMethod` is
// well-defined for adding a method to an already-registered class; it
// doesn't require the class to be freshly declared.
//
// Everything here is done via raw C FFI against libobjc, deliberately
// avoiding the objc2 crate family, to avoid a repeat of the winit-delegate
// situation.
//
// Ported from https://github.com/Barni228/slint_mac_open_with, adapted so
// `report_paths` forwards a single path at a time (matching this app's
// one-file-at-a-time `analyze_file` flow) instead of joining multiple
// selected files into one comma-separated display string.
//
// Logging: every step below is logged both to stderr AND to a plain log
// file at ~/Library/Logs/video-inspector-open-with.log. When Finder/Launch
// Services cold-launches the app, the new process has no terminal attached
// at all -- stderr from that instance won't show up anywhere you're
// watching unless you tail the log file instead:
//   tail -f ~/Library/Logs/video-inspector-open-with.log

#![cfg(target_os = "macos")]

use std::ffi::{CStr, CString, c_char, c_void};
use std::io::Write;
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender, channel};

pub fn log_line(msg: &str) {
    eprintln!("{msg}");
    let _ = (|| -> std::io::Result<()> {
        let home = std::env::var("HOME").map_err(|_| std::io::ErrorKind::NotFound)?;
        let log_dir = std::path::Path::new(&home).join("Library/Logs");
        std::fs::create_dir_all(&log_dir)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("video-inspector-open-with.log"))?;
        writeln!(file, "{msg}")
    })();
}

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
            log_line("[macos_open] (swizzle) could not find NSApplication class");
            return;
        }

        let shared_app: Id = msg_send0(ns_app_class, sel("sharedApplication"));
        if shared_app.is_null() {
            log_line("[macos_open] (swizzle) sharedApplication returned nil");
            return;
        }

        let delegate: Id = msg_send0(shared_app, sel("delegate"));
        if delegate.is_null() {
            log_line(
                "[macos_open] (swizzle) NSApp has no delegate yet - call this after AppWindow::new()",
            );
            return;
        }

        let delegate_class = object_getClass(delegate);
        if delegate_class.is_null() {
            log_line("[macos_open] (swizzle) could not get delegate's class");
            return;
        }

        let selector = sel("application:openURLs:");
        // "v@:@@" = void return; self, _cmd, and two object args.
        let types = CString::new("v@:@@").unwrap();
        let imp: Imp = std::mem::transmute(application_open_urls as *const ());
        let added = class_addMethod(delegate_class, selector, imp, types.as_ptr());
        std::mem::forget(types);

        log_line(&format!(
            "[macos_open] (swizzle) class_addMethod application:openURLs: -> added={added}"
        ));
    }
}

/// The IMP for `application:openURLs:`, matching the ObjC method signature
/// `- (void)application:(NSApplication *)application openURLs:(NSArray<NSURL *> *)urls;`
/// i.e. C signature `void fn(id self, SEL _cmd, id application, id urls)`.
///
/// Wrapped in catch_unwind: this is called directly by AppKit across the ObjC
/// ABI boundary, and Rust aborts the whole process if a panic unwinds past an
/// `extern "C" fn` -- better to log and drop this one event than take the app
/// down over a single unexpected URL.
extern "C" fn application_open_urls(_this: Id, _cmd: Sel, _application: Id, urls: Id) {
    let result = std::panic::catch_unwind(|| unsafe {
        if urls.is_null() {
            log_line("[macos_open] (swizzle) application:openURLs: called with nil array");
            return;
        }

        let count: u64 = msg_send0(urls, sel("count"));
        log_line(&format!(
            "[macos_open] (swizzle) application:openURLs: called with {count} url(s)"
        ));

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
            log_line(&format!("[macos_open] (swizzle) url {i} -> {path}"));
            paths.push(path);
        }

        report_paths(paths);
    });
    if result.is_err() {
        log_line("[macos_open] application_open_urls panicked - see above for details");
    }
}
