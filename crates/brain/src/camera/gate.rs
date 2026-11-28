use nixa_phone::CameraPhone;

pub const NO_PHONE: &str = "No phone with Nixa Mobile is connected. Turn Nixa on in Nixa Mobile on the phone.";

fn same(phone: &CameraPhone, asked: &str) -> bool {
    let asked = asked.trim().to_lowercase();
    phone.id.to_lowercase() == asked || phone.name.trim().to_lowercase() == asked
}

pub fn pick(phones: &[CameraPhone], asked: Option<&str>) -> Result<CameraPhone, String> {
    let first = phones.first().ok_or_else(|| NO_PHONE.to_string())?;
    match asked.map(str::trim).filter(|p| !p.is_empty()) {
        Some(asked) => phones.iter().find(|p| same(p, asked)).cloned().ok_or_else(|| format!("No phone named {asked:?} is connected. Connected phones: {}.", names(phones))),
        None => Ok(first.clone()),
    }
}

fn names(phones: &[CameraPhone]) -> String {
    phones.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", ")
}
