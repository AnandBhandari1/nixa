use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Outside,
    Menu,
    WindowClosed,
    Timer,
    NoWindow,
}

static WHY: AtomicU8 = AtomicU8::new(0);

fn code(why: Why) -> u8 {
    match why {
        Why::Outside => 0,
        Why::Menu => 1,
        Why::WindowClosed => 2,
        Why::Timer => 3,
        Why::NoWindow => 4,
    }
}

fn from_code(code: u8) -> Why {
    match code {
        1 => Why::Menu,
        2 => Why::WindowClosed,
        3 => Why::Timer,
        4 => Why::NoWindow,
        _ => Why::Outside,
    }
}

pub fn mark_in(slot: &AtomicU8, why: Why) {
    slot.compare_exchange(0, code(why), Ordering::SeqCst, Ordering::SeqCst)
        .ok();
}

pub fn read_in(slot: &AtomicU8) -> Why {
    from_code(slot.load(Ordering::SeqCst))
}

pub fn mark(why: Why) {
    mark_in(&WHY, why);
}

pub fn why() -> Why {
    read_in(&WHY)
}

pub fn bundle_of(exe: &std::path::Path) -> Option<std::path::PathBuf> {
    exe.ancestors()
        .find(|dir| dir.extension().is_some_and(|ext| ext == "app"))
        .map(std::path::Path::to_path_buf)
}

pub fn comes_back(why: Why, bundle: Option<&std::path::Path>, nixa_on: bool) -> bool {
    why == Why::Outside && bundle.is_some() && nixa_on
}

pub fn come_back_after_outside_quit() {
    let bundle = std::env::current_exe().ok().and_then(|exe| bundle_of(&exe));
    let nixa_on = !crate::screens::nixa_screen::host::off_now();
    if !comes_back(why(), bundle.as_deref(), nixa_on) {
        return;
    }
    let Some(bundle) = bundle else { return };
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.args(["-c", "sleep 3; exec /usr/bin/open \"$0\""]).arg(&bundle);
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    match cmd.spawn() {
        Ok(_) => eprintln!("nixa-app: Nixa is on, so the app opens again in 3 s after the quit from outside"),
        Err(e) => eprintln!("nixa-app: could not open the app again after the quit from outside: {e}"),
    }
}

const SENDER_PID_KEY: u32 = u32::from_be_bytes(*b"spid");

static SENDER: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

#[cfg(target_os = "macos")]
mod quit_event {
    use std::ffi::c_void;
    use std::sync::OnceLock;

    #[repr(C, packed(2))]
    pub struct AEDesc {
        pub descriptor_type: u32,
        pub data_handle: *mut c_void,
    }

    pub type Handler = extern "C" fn(*const AEDesc, *mut AEDesc, *mut c_void) -> i16;

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn AEGetEventHandler(class: u32, id: u32, handler: *mut Option<Handler>, refcon: *mut *mut c_void, system: u8) -> i16;
        pub fn AEInstallEventHandler(class: u32, id: u32, handler: Handler, refcon: *mut c_void, system: u8) -> i16;
        fn AEGetAttributePtr(event: *const AEDesc, key: u32, want: u32, got: *mut u32, data: *mut c_void, max: isize, size: *mut isize) -> i16;
    }

    pub const CORE: u32 = u32::from_be_bytes(*b"aevt");
    pub const QUIT: u32 = u32::from_be_bytes(*b"quit");
    pub const SINT32: u32 = u32::from_be_bytes(*b"long");

    static APPKIT: OnceLock<(Handler, usize)> = OnceLock::new();

    pub fn pid_of(event: *const AEDesc) -> Option<i32> {
        let mut pid: i32 = 0;
        let (mut got, mut size) = (0u32, 0isize);
        let err = unsafe {
            AEGetAttributePtr(event, super::SENDER_PID_KEY, SINT32, &mut got, (&mut pid as *mut i32).cast(), 4, &mut size)
        };
        (err == 0 && pid > 0).then_some(pid)
    }

    extern "C" fn on_quit(event: *const AEDesc, reply: *mut AEDesc, _: *mut c_void) -> i16 {
        super::SENDER.store(pid_of(event).unwrap_or(0), std::sync::atomic::Ordering::SeqCst);
        match APPKIT.get() {
            Some((handler, refcon)) => handler(event, reply, *refcon as *mut c_void),
            None => -1708,
        }
    }

    pub fn watch() {
        let mut handler: Option<Handler> = None;
        let mut refcon: *mut c_void = std::ptr::null_mut();
        let found = unsafe { AEGetEventHandler(CORE, QUIT, &mut handler, &mut refcon, 0) };
        let Some(handler) = handler.filter(|_| found == 0) else {
            eprintln!("nixa-app: no quit Apple Event handler to watch, so the quit sender is not logged");
            return;
        };
        if handler as usize == on_quit as Handler as usize || APPKIT.set((handler, refcon as usize)).is_err() {
            return;
        }
        let err = unsafe { AEInstallEventHandler(CORE, QUIT, on_quit, std::ptr::null_mut(), 0) };
        if err != 0 {
            eprintln!("nixa-app: cannot watch the quit Apple Event (error {err})");
        }
    }
}

#[cfg(target_os = "macos")]
pub fn watch_quit_event() {
    quit_event::watch();
}

#[cfg(not(target_os = "macos"))]
pub fn watch_quit_event() {}

fn sender_pid() -> Option<i32> {
    let seen = SENDER.load(Ordering::SeqCst);
    if seen > 0 {
        return Some(seen);
    }
    current_sender_pid()
}

#[cfg(target_os = "macos")]
fn current_sender_pid() -> Option<i32> {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    unsafe {
        let manager: *mut AnyObject = msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
        if manager.is_null() {
            return None;
        }
        let event: *mut AnyObject = msg_send![manager, currentAppleEvent];
        if event.is_null() {
            return None;
        }
        let desc: *mut AnyObject = msg_send![event, attributeDescriptorForKeyword: SENDER_PID_KEY];
        if desc.is_null() {
            return None;
        }
        let pid: i32 = msg_send![desc, int32Value];
        (pid > 0).then_some(pid)
    }
}

#[cfg(not(target_os = "macos"))]
fn current_sender_pid() -> Option<i32> {
    None
}

#[cfg(target_os = "macos")]
fn path_of(pid: i32) -> Option<String> {
    let mut buf = vec![0u8; 4096];
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

#[cfg(not(target_os = "macos"))]
fn path_of(_pid: i32) -> Option<String> {
    None
}

pub fn sender_line(pid: Option<i32>, path: Option<&str>) -> String {
    match (pid, path) {
        (Some(pid), Some(path)) => format!("nixa-app: the quit Apple Event came from pid {pid} ({path})"),
        (Some(pid), None) => format!("nixa-app: the quit Apple Event came from pid {pid} (program gone)"),
        _ => "nixa-app: the quit sender is not known (no Apple Event with a sender pid)".to_string(),
    }
}

pub fn log_sender() {
    if why() != Why::Outside {
        return;
    }
    let pid = sender_pid();
    eprintln!("{}", sender_line(pid, pid.and_then(path_of).as_deref()));
}

pub fn line(why: Why) -> &'static str {
    match why {
        Why::Outside => {
            "nixa-app: quit was asked from outside the app (Dock menu, a quit Apple Event or log out)"
        }
        Why::Menu => "nixa-app: quit from the Nixa menu or Cmd+Q",
        Why::WindowClosed => "nixa-app: the main window was closed, so the app quits",
        Why::Timer => "nixa-app: NIXA_APP_QUIT_AFTER_MS time is up, so the app quits",
        Why::NoWindow => "nixa-app: the main window did not open, so the app quits",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quit_nobody_in_the_app_asked_for_comes_from_outside() {
        let slot = AtomicU8::new(0);
        assert_eq!(read_in(&slot), Why::Outside);
        assert!(line(read_in(&slot)).contains("Dock menu"));
    }

    #[test]
    fn only_an_outside_quit_with_nixa_on_comes_back() {
        let exe = std::path::Path::new("/Applications/Nixa.app/Contents/MacOS/nixa-app");
        let bundle = bundle_of(exe);
        assert_eq!(bundle.as_deref(), Some(std::path::Path::new("/Applications/Nixa.app")));
        assert!(comes_back(Why::Outside, bundle.as_deref(), true));
        assert!(!comes_back(Why::Menu, bundle.as_deref(), true));
        assert!(!comes_back(Why::Outside, bundle.as_deref(), false));
        assert!(!comes_back(Why::Outside, bundle_of(std::path::Path::new("/tmp/target/debug/nixa-app")).as_deref(), true));
    }

    #[test]
    fn the_quit_sender_line_names_the_pid_and_program() {
        assert_eq!(u32::from_be_bytes(*b"spid"), 0x7370_6964);
        assert!(sender_line(Some(42), Some("/usr/bin/osascript")).contains("pid 42 (/usr/bin/osascript)"));
        assert!(sender_line(None, None).contains("not known"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_sender_pid_is_read_from_the_quit_event() {
        use quit_event::*;
        #[link(name = "CoreServices", kind = "framework")]
        unsafe extern "C" {
            fn AECreateDesc(kind: u32, data: *const std::ffi::c_void, size: isize, out: *mut AEDesc) -> i16;
            fn AECreateAppleEvent(class: u32, id: u32, target: *const AEDesc, ret: i16, tx: i32, out: *mut AEDesc) -> i16;
            fn AESendMessage(event: *const AEDesc, reply: *mut AEDesc, mode: i32, timeout: isize) -> i32;
            fn AEDisposeDesc(desc: *mut AEDesc) -> i16;
        }
        static SEEN: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
        extern "C" fn probe(event: *const AEDesc, _: *mut AEDesc, _: *mut std::ffi::c_void) -> i16 {
            SEEN.store(pid_of(event).unwrap_or(-1), Ordering::SeqCst);
            0
        }
        let class = u32::from_be_bytes(*b"nxts");
        let psn: [u32; 2] = [0, 2];
        let mut target = AEDesc { descriptor_type: 0, data_handle: std::ptr::null_mut() };
        let mut event = AEDesc { descriptor_type: 0, data_handle: std::ptr::null_mut() };
        let mut reply = AEDesc { descriptor_type: 0, data_handle: std::ptr::null_mut() };
        unsafe {
            assert_eq!(AEInstallEventHandler(class, QUIT, probe, std::ptr::null_mut(), 0), 0);
            assert_eq!(AECreateDesc(u32::from_be_bytes(*b"psn "), psn.as_ptr().cast(), 8, &mut target), 0);
            assert_eq!(AECreateAppleEvent(class, QUIT, &target, -1, 0, &mut event), 0);
            assert_eq!(AESendMessage(&event, &mut reply, 3, 600), 0);
            AEDisposeDesc(&mut reply);
            AEDisposeDesc(&mut event);
            AEDisposeDesc(&mut target);
        }
        assert_eq!(SEEN.load(Ordering::SeqCst), std::process::id() as i32);
    }

    #[test]
    fn the_first_reason_stays() {
        let slot = AtomicU8::new(0);
        mark_in(&slot, Why::Menu);
        mark_in(&slot, Why::WindowClosed);
        assert_eq!(read_in(&slot), Why::Menu);
    }

    #[test]
    fn every_reason_has_its_own_line() {
        let all = [
            Why::Outside,
            Why::Menu,
            Why::WindowClosed,
            Why::Timer,
            Why::NoWindow,
        ];
        for why in all {
            let slot = AtomicU8::new(0);
            mark_in(&slot, why);
            assert_eq!(read_in(&slot), why);
            assert!(line(why).starts_with("nixa-app: "));
        }
        let mut lines: Vec<&str> = all.iter().map(|w| line(*w)).collect();
        lines.sort();
        lines.dedup();
        assert_eq!(lines.len(), all.len());
    }
}
