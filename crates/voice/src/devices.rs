use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevice {
    pub name: String,
    pub default: bool,
}

pub fn pick_name(names: &[String], wanted: &str) -> Option<usize> {
    let want = wanted.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    names
        .iter()
        .position(|n| n.to_lowercase() == want)
        .or_else(|| names.iter().position(|n| n.to_lowercase().contains(&want)))
}

pub(crate) fn device_name(device: &cpal::Device) -> String {
    device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "unknown".to_string())
}

fn list(input: bool) -> Result<Vec<AudioDevice>> {
    let host = cpal::default_host();
    let default = if input { host.default_input_device() } else { host.default_output_device() }.map(|d| device_name(&d));
    let devices: Vec<cpal::Device> = if input {
        host.input_devices().context("list input devices")?.collect()
    } else {
        host.output_devices().context("list output devices")?.collect()
    };
    Ok(devices
        .iter()
        .map(|d| {
            let name = device_name(d);
            let default = default.as_deref() == Some(name.as_str());
            AudioDevice { name, default }
        })
        .collect())
}

pub fn list_input_devices() -> Result<Vec<AudioDevice>> {
    list(true)
}

pub fn list_output_devices() -> Result<Vec<AudioDevice>> {
    list(false)
}

pub(crate) fn find_device(input: bool, wanted: Option<&str>) -> Result<(cpal::Device, bool)> {
    let host = cpal::default_host();
    let fallback = || {
        if input { host.default_input_device() } else { host.default_output_device() }
            .context(if input { "no microphone found" } else { "no sound output found" })
    };
    let Some(wanted) = wanted.map(str::trim).filter(|w| !w.is_empty()) else {
        return Ok((fallback()?, false));
    };
    let devices: Vec<cpal::Device> = if input {
        host.input_devices().context("list input devices")?.collect()
    } else {
        host.output_devices().context("list output devices")?.collect()
    };
    let names: Vec<String> = devices.iter().map(device_name).collect();
    match pick_name(&names, wanted) {
        Some(i) => Ok((devices[i].clone(), false)),
        None => Ok((fallback()?, true)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_name_prefers_exact_then_part() {
        let names = vec!["MacBook Pro Microphone".to_string(), "AirPods Pro".to_string(), "Pro".to_string()];
        assert_eq!(pick_name(&names, "pro"), Some(2));
        assert_eq!(pick_name(&names, "airpods"), Some(1));
        assert_eq!(pick_name(&names, "  "), None);
        assert_eq!(pick_name(&names, "usb mic"), None);
    }
}
