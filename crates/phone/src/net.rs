use crate::cert::DoorNames;
use std::ffi::CStr;
use std::net::Ipv4Addr;
use std::process::Command;

pub fn is_private_ip(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    a == 10 || (a == 192 && b == 168) || (a == 172 && (16..=31).contains(&b))
}

fn wired_name(name: &str) -> bool {
    ["en", "eth", "wlan"].iter().any(|start| {
        name.strip_prefix(start).is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
    })
}

pub fn pick_address(list: &[(String, Ipv4Addr)]) -> Option<Ipv4Addr> {
    let usable = |name: &str| list.iter().find(|(n, ip)| n == name && is_private_ip(*ip)).map(|(_, ip)| *ip);
    if let Some(ip) = usable("en0") {
        return Some(ip);
    }
    list.iter().find(|(n, ip)| wired_name(n) && is_private_ip(*ip)).map(|(_, ip)| *ip)
}

pub fn interfaces() -> Vec<(String, Ipv4Addr)> {
    let mut out = Vec::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return out;
    }
    let mut at = head;
    while !at.is_null() {
        let item = unsafe { &*at };
        at = item.ifa_next;
        if item.ifa_addr.is_null() || item.ifa_name.is_null() {
            continue;
        }
        if item.ifa_flags & (libc::IFF_LOOPBACK as u32) != 0 || item.ifa_flags & (libc::IFF_UP as u32) == 0 {
            continue;
        }
        let family = unsafe { (*item.ifa_addr).sa_family };
        if i32::from(family) != libc::AF_INET {
            continue;
        }
        let addr = unsafe { &*(item.ifa_addr as *const libc::sockaddr_in) };
        let ip = Ipv4Addr::from(u32::from_be(addr.sin_addr.s_addr));
        let name = unsafe { CStr::from_ptr(item.ifa_name) }.to_string_lossy().into_owned();
        out.push((name, ip));
    }
    unsafe { libc::freeifaddrs(head) };
    out
}

pub fn wifi_address() -> Option<String> {
    pick_address(&interfaces()).map(|ip| ip.to_string())
}

pub fn local_name() -> Option<String> {
    let out = Command::new("/usr/sbin/scutil").args(["--get", "LocalHostName"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    good_host_label(&name).then(|| format!("{name}.local"))
}

pub fn good_host_label(name: &str) -> bool {
    !name.is_empty() && name.len() <= 63 && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub fn door_names(address: &str) -> DoorNames {
    DoorNames { ips: vec![address.to_string()], dns: local_name().into_iter().collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> Ipv4Addr {
        text.parse().unwrap()
    }

    #[test]
    fn only_private_wifi_addresses_are_picked() {
        assert!(is_private_ip(ip("10.1.2.3")));
        assert!(is_private_ip(ip("172.16.0.1")));
        assert!(is_private_ip(ip("172.31.255.1")));
        assert!(!is_private_ip(ip("172.32.0.1")));
        assert!(is_private_ip(ip("192.168.0.9")));
        assert!(!is_private_ip(ip("8.8.8.8")));
        assert!(!is_private_ip(ip("127.0.0.1")));
        let list = vec![
            ("utun3".to_string(), ip("10.8.0.2")),
            ("en5".to_string(), ip("192.168.2.4")),
            ("en0".to_string(), ip("8.8.8.8")),
        ];
        assert_eq!(pick_address(&list), Some(ip("192.168.2.4")));
        let list = vec![("en5".to_string(), ip("192.168.2.4")), ("en0".to_string(), ip("192.168.1.9"))];
        assert_eq!(pick_address(&list), Some(ip("192.168.1.9")));
        assert_eq!(pick_address(&[("bridge0".to_string(), ip("192.168.64.1"))]), None);
        assert!(wired_name("wlan0"));
        assert!(!wired_name("en"));
        assert!(!wired_name("enx1"));
    }

    #[test]
    fn host_labels_are_checked() {
        assert!(good_host_label("My-Mac-mini"));
        assert!(!good_host_label("bad name"));
        assert!(!good_host_label(""));
    }

    #[test]
    fn interfaces_can_be_read() {
        let list = interfaces();
        assert!(list.iter().all(|(_, ip)| !ip.is_loopback()));
    }
}
