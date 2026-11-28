use anyhow::Result;
use nixa_store::Connection;

pub const KEEP_KEY: &str = "nixa_screen.keep_open";
pub const QUIET_KEY: &str = "nixa_screen.quiet";
pub const LOOK_KEY: &str = "nixa_screen.look";
pub const STILL_KEY: &str = "nixa_screen.camera_still";
pub const MUTE_KEY: &str = "nixa_screen.office_mute";
pub const SOAP_KEY: &str = "nixa_screen.office_soap";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Look {
    #[default]
    Universe,
    Office,
}

impl Look {
    pub const ALL: [Look; 2] = [Look::Universe, Look::Office];

    pub fn word(self) -> &'static str {
        match self {
            Look::Universe => "universe",
            Look::Office => "office",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Look::Universe => "Universe",
            Look::Office => "Office",
        }
    }

    pub fn from_word(word: &str) -> Look {
        Look::ALL
            .into_iter()
            .find(|l| l.word() == word.trim())
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Prefs {
    pub keep_open: bool,
    pub quiet: bool,
    pub look: Look,
    pub still: bool,
    pub mute: bool,
    pub soap: bool,
}

pub fn load(conn: Option<&Connection>) -> Prefs {
    let Some(conn) = conn else {
        return Prefs::default();
    };
    Prefs {
        keep_open: nixa_store::settings::setting_bool(conn, KEEP_KEY, false).unwrap_or(false),
        quiet: nixa_store::settings::setting_bool(conn, QUIET_KEY, false).unwrap_or(false),
        look: nixa_store::settings::setting_text(conn, LOOK_KEY, "universe")
            .map(|w| Look::from_word(&w))
            .unwrap_or_default(),
        still: nixa_store::settings::setting_bool(conn, STILL_KEY, false).unwrap_or(false),
        mute: nixa_store::settings::setting_bool(conn, MUTE_KEY, false).unwrap_or(false),
        soap: nixa_store::settings::setting_bool(conn, SOAP_KEY, false).unwrap_or(false),
    }
}

pub fn save(conn: Option<&Connection>, prefs: Prefs) -> Result<()> {
    let conn = conn.ok_or_else(|| {
        anyhow::anyhow!("The store is not open, so this choice is kept until the app closes.")
    })?;
    nixa_store::settings::set_setting_bool(conn, KEEP_KEY, prefs.keep_open)?;
    nixa_store::settings::set_setting_bool(conn, QUIET_KEY, prefs.quiet)?;
    nixa_store::settings::set_setting_text(conn, LOOK_KEY, prefs.look.word())?;
    nixa_store::settings::set_setting_bool(conn, STILL_KEY, prefs.still)?;
    nixa_store::settings::set_setting_bool(conn, MUTE_KEY, prefs.mute)?;
    nixa_store::settings::set_setting_bool(conn, SOAP_KEY, prefs.soap)?;
    Ok(())
}
