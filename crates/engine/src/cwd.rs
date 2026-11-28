use std::path::Path;

#[cfg(target_os = "macos")]
pub fn of_pid(pid: u32) -> Option<String> {
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    let got = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if got != size {
        return None;
    }
    let raw = &info.pvi_cdir.vip_path;
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(raw.as_ptr().cast(), std::mem::size_of_val(raw)) };
    let text = std::ffi::CStr::from_bytes_until_nul(bytes).ok()?.to_str().ok()?;
    usable(text)
}

#[cfg(not(target_os = "macos"))]
pub fn of_pid(pid: u32) -> Option<String> {
    let path = std::fs::read_link(format!("/proc/{pid}/cwd")).ok()?;
    usable(path.to_str()?)
}

fn usable(path: &str) -> Option<String> {
    (!path.is_empty() && Path::new(path).is_absolute()).then(|| path.to_string())
}

pub fn pick(live: Option<&str>, start: &str) -> String {
    live.filter(|c| Path::new(c).is_dir()).unwrap_or(start).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_folder_is_read() {
        let here = std::env::current_dir().unwrap().canonicalize().unwrap();
        let seen = of_pid(std::process::id()).unwrap();
        assert_eq!(Path::new(&seen).canonicalize().unwrap(), here);
    }

    #[test]
    fn a_dead_pid_gives_nothing() {
        assert_eq!(of_pid(0x7fff_fff0), None);
    }

    #[test]
    fn pick_falls_back_to_the_start_folder() {
        let tmp = std::env::temp_dir().display().to_string();
        assert_eq!(pick(Some(&tmp), "/"), tmp);
        assert_eq!(pick(Some("/no/such/folder"), "/"), "/");
        assert_eq!(pick(None, "/"), "/");
    }
}
