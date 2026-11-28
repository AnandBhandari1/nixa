use crate::compile::{BINS, MAIN_BIN};
use crate::layout::macos_dir;
use crate::opts::{AD_HOC, BUNDLE_ID};
use crate::run;
use std::path::Path;
use std::process::Command;

const CODESIGN: &str = "/usr/bin/codesign";
pub const LOCAL_IDENTITY: &str = "AutoNotes Local";

pub fn pick_identity(listing: &str) -> String {
    let wanted = format!("\"{LOCAL_IDENTITY}\"");
    if listing.lines().any(|line| line.contains(&wanted)) { LOCAL_IDENTITY.to_string() } else { AD_HOC.to_string() }
}

pub fn default_identity() -> String {
    let listing = run::text(Command::new("/usr/bin/security").args(["find-identity", "-p", "codesigning"])).unwrap_or_default();
    pick_identity(&listing)
}

pub fn identifier(bin: &str) -> String {
    if bin == MAIN_BIN { BUNDLE_ID.to_string() } else { format!("{BUNDLE_ID}.{bin}") }
}

fn codesign(identity: &str, id: &str, path: &Path) -> Result<(), String> {
    run::text(Command::new(CODESIGN).args(["--force", "--sign", identity, "--identifier", id]).arg(path)).map(|_| ())
}

pub fn sign(app: &Path, identity: &str) -> Result<(), String> {
    for (_, bin) in BINS.iter().filter(|(_, bin)| *bin != MAIN_BIN) {
        codesign(identity, &identifier(bin), &macos_dir(app).join(bin))?;
    }
    codesign(identity, BUNDLE_ID, app)
}

pub fn field<'a>(info: &'a str, name: &str) -> Option<&'a str> {
    info.lines().find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
}

pub fn verify(app: &Path) -> Result<String, String> {
    run::text(Command::new(CODESIGN).args(["--verify", "--deep", "--strict"]).arg(app))?;
    let info = run::text(Command::new(CODESIGN).arg("-dv").arg(app))?;
    if field(&info, "Identifier") != Some(BUNDLE_ID) {
        return Err(format!("the bundle is not signed as {BUNDLE_ID}: {}", info.trim()));
    }
    for (_, bin) in BINS.iter().filter(|(_, bin)| *bin != MAIN_BIN) {
        let path = macos_dir(app).join(bin);
        let info = run::text(Command::new(CODESIGN).arg("-dv").arg(&path))?;
        if field(&info, "Identifier") != Some(identifier(bin).as_str()) {
            return Err(format!("{bin} is not signed as {}", identifier(bin)));
        }
    }
    let kind = field(&info, "Signature").or_else(|| field(&info, "Authority")).unwrap_or("signed");
    Ok(format!("signed as {BUNDLE_ID} ({kind})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{assemble, stage_path};
    use crate::testkit::{fake_release, fake_root, temp};

    #[test]
    fn names_follow_the_bundle_id() {
        assert_eq!(identifier("nixa-app"), "dev.nixa.app");
        assert_eq!(identifier("nixa"), "dev.nixa.app.nixa");
        assert_eq!(identifier("nixa-brain"), "dev.nixa.app.nixa-brain");
        let info = "Executable=/x\nIdentifier=dev.nixa.app\nSignature=adhoc\n";
        assert_eq!(field(info, "Identifier"), Some("dev.nixa.app"));
        assert_eq!(field(info, "Signature"), Some("adhoc"));
        assert_eq!(field(info, "TeamIdentifier"), None);
    }

    #[test]
    fn the_local_identity_is_picked_only_when_listed() {
        let listed = "  1) F4A8 \"AutoNotes Local\" (CSSMERR_TP_NOT_TRUSTED)\n     1 identities found\n";
        assert_eq!(pick_identity(listed), "AutoNotes Local");
        assert_eq!(pick_identity("     0 identities found\n"), "-");
    }

    #[test]
    fn an_ad_hoc_signed_bundle_verifies() {
        let dir = temp("sign");
        let root = fake_root(&dir);
        let release = fake_release(&dir);
        let stage = stage_path(&dir.join("out"));
        assemble(&root, &release, &stage, "0.1.0").unwrap();
        assert!(verify(&stage).is_err());
        sign(&stage, "-").unwrap();
        assert_eq!(verify(&stage).unwrap(), "signed as dev.nixa.app (adhoc)");
        std::fs::write(crate::layout::resources_dir(&stage).join("hooks/nixa-status.sh"), "changed").unwrap();
        assert!(verify(&stage).is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
