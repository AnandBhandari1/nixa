use anyhow::{Result, bail};

pub const PHONE_PORT: u16 = 7430;
pub const SETUP_PORT: u16 = 7431;
pub const REFUSED: [u16; 4] = [7321, 7330, 7331, 7421];

pub fn check_ports(port: u16, setup: u16) -> Result<()> {
    for p in [port, setup] {
        if REFUSED.contains(&p) {
            bail!("port {p} is not allowed for the phone door");
        }
    }
    if port != 0 && port == setup {
        bail!("the phone door and the setup page need two different ports");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_ports_are_refused() {
        assert!(check_ports(7330, 7431).is_err());
        assert!(check_ports(7430, 7331).is_err());
        assert!(check_ports(7321, 7431).is_err());
        assert!(check_ports(7421, 7431).is_err());
        assert!(check_ports(7430, 7430).is_err());
        assert!(check_ports(PHONE_PORT, SETUP_PORT).is_ok());
        assert!(check_ports(0, 0).is_ok());
    }
}
