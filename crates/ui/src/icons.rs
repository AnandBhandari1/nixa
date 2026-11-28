use gpui::{
    Animation, AnimationExt, AssetSource, ElementId, Hsla, IntoElement, Result, SharedString, Styled,
    Svg, Transformation, percentage, px, svg,
};
use std::borrow::Cow;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Archive,
    ArrowDown,
    ArrowUp,
    Bell,
    Chat,
    Check,
    ChevronDown,
    ChevronRight,
    CircleAlert,
    CircleCheck,
    CircleDot,
    CircleX,
    Circle,
    Clock,
    Commands,
    Copy,
    Core,
    Diamond,
    Ear,
    Eraser,
    Files,
    FolderPlus,
    Folder,
    GitBranch,
    Hexagon,
    Info,
    Layers,
    Lock,
    Menu,
    MicOff,
    Mic,
    Minus,
    Monitor,
    Notebook,
    PanelRight,
    Pin,
    Play,
    Plus,
    Refresh,
    Search,
    Send,
    Settings,
    Shield,
    Sidebar,
    Smartphone,
    Spinner,
    SquarePen,
    Square,
    Star,
    Tasks,
    Terminal,
    Trash,
    Triangle,
    Undo,
    Unlock,
    Users,
    VoiceOff,
    VoiceOn,
    VolumeOff,
    Volume,
    X,
    Zap,
}

impl Icon {
    pub const ALL: [Icon; 62] = [
        Icon::Archive,
        Icon::ArrowDown,
        Icon::ArrowUp,
        Icon::Bell,
        Icon::Chat,
        Icon::Check,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::CircleAlert,
        Icon::CircleCheck,
        Icon::CircleDot,
        Icon::CircleX,
        Icon::Circle,
        Icon::Clock,
        Icon::Commands,
        Icon::Copy,
        Icon::Core,
        Icon::Diamond,
        Icon::Ear,
        Icon::Eraser,
        Icon::Files,
        Icon::FolderPlus,
        Icon::Folder,
        Icon::GitBranch,
        Icon::Hexagon,
        Icon::Info,
        Icon::Layers,
        Icon::Lock,
        Icon::Menu,
        Icon::MicOff,
        Icon::Mic,
        Icon::Minus,
        Icon::Monitor,
        Icon::Notebook,
        Icon::PanelRight,
        Icon::Pin,
        Icon::Play,
        Icon::Plus,
        Icon::Refresh,
        Icon::Search,
        Icon::Send,
        Icon::Settings,
        Icon::Shield,
        Icon::Sidebar,
        Icon::Smartphone,
        Icon::Spinner,
        Icon::SquarePen,
        Icon::Square,
        Icon::Star,
        Icon::Tasks,
        Icon::Terminal,
        Icon::Trash,
        Icon::Triangle,
        Icon::Undo,
        Icon::Unlock,
        Icon::Users,
        Icon::VoiceOff,
        Icon::VoiceOn,
        Icon::VolumeOff,
        Icon::Volume,
        Icon::X,
        Icon::Zap,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Icon::Archive => "archive",
            Icon::ArrowDown => "arrow-down",
            Icon::ArrowUp => "arrow-up",
            Icon::Bell => "bell",
            Icon::Chat => "chat",
            Icon::Check => "check",
            Icon::ChevronDown => "chevron-down",
            Icon::ChevronRight => "chevron-right",
            Icon::CircleAlert => "circle-alert",
            Icon::CircleCheck => "circle-check",
            Icon::CircleDot => "circle-dot",
            Icon::CircleX => "circle-x",
            Icon::Circle => "circle",
            Icon::Clock => "clock",
            Icon::Commands => "commands",
            Icon::Copy => "copy",
            Icon::Core => "core",
            Icon::Diamond => "diamond",
            Icon::Ear => "ear",
            Icon::Eraser => "eraser",
            Icon::Files => "files",
            Icon::FolderPlus => "folder-plus",
            Icon::Folder => "folder",
            Icon::GitBranch => "git-branch",
            Icon::Hexagon => "hexagon",
            Icon::Info => "info",
            Icon::Layers => "layers",
            Icon::Lock => "lock",
            Icon::Menu => "menu",
            Icon::MicOff => "mic-off",
            Icon::Mic => "mic",
            Icon::Minus => "minus",
            Icon::Monitor => "monitor",
            Icon::Notebook => "notebook",
            Icon::PanelRight => "panel-right",
            Icon::Pin => "pin",
            Icon::Play => "play",
            Icon::Plus => "plus",
            Icon::Refresh => "refresh",
            Icon::Search => "search",
            Icon::Send => "send",
            Icon::Settings => "settings",
            Icon::Shield => "shield",
            Icon::Sidebar => "sidebar",
            Icon::Smartphone => "smartphone",
            Icon::Spinner => "spinner",
            Icon::SquarePen => "square-pen",
            Icon::Square => "square",
            Icon::Star => "star",
            Icon::Tasks => "tasks",
            Icon::Terminal => "terminal",
            Icon::Trash => "trash",
            Icon::Triangle => "triangle",
            Icon::Undo => "undo",
            Icon::Unlock => "unlock",
            Icon::Users => "users",
            Icon::VoiceOff => "voice-off",
            Icon::VoiceOn => "voice-on",
            Icon::VolumeOff => "volume-off",
            Icon::Volume => "volume",
            Icon::X => "x",
            Icon::Zap => "zap",
        }
    }

    pub fn path(self) -> SharedString {
        format!("icons/{}.svg", self.name()).into()
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Icon::Archive => include_bytes!("../assets/icons/archive.svg"),
            Icon::ArrowDown => include_bytes!("../assets/icons/arrow-down.svg"),
            Icon::ArrowUp => include_bytes!("../assets/icons/arrow-up.svg"),
            Icon::Bell => include_bytes!("../assets/icons/bell.svg"),
            Icon::Chat => include_bytes!("../assets/icons/chat.svg"),
            Icon::Check => include_bytes!("../assets/icons/check.svg"),
            Icon::ChevronDown => include_bytes!("../assets/icons/chevron-down.svg"),
            Icon::ChevronRight => include_bytes!("../assets/icons/chevron-right.svg"),
            Icon::CircleAlert => include_bytes!("../assets/icons/circle-alert.svg"),
            Icon::CircleCheck => include_bytes!("../assets/icons/circle-check.svg"),
            Icon::CircleDot => include_bytes!("../assets/icons/circle-dot.svg"),
            Icon::CircleX => include_bytes!("../assets/icons/circle-x.svg"),
            Icon::Circle => include_bytes!("../assets/icons/circle.svg"),
            Icon::Clock => include_bytes!("../assets/icons/clock.svg"),
            Icon::Commands => include_bytes!("../assets/icons/commands.svg"),
            Icon::Copy => include_bytes!("../assets/icons/copy.svg"),
            Icon::Core => include_bytes!("../assets/icons/core.svg"),
            Icon::Diamond => include_bytes!("../assets/icons/diamond.svg"),
            Icon::Ear => include_bytes!("../assets/icons/ear.svg"),
            Icon::Eraser => include_bytes!("../assets/icons/eraser.svg"),
            Icon::Files => include_bytes!("../assets/icons/files.svg"),
            Icon::FolderPlus => include_bytes!("../assets/icons/folder-plus.svg"),
            Icon::Folder => include_bytes!("../assets/icons/folder.svg"),
            Icon::GitBranch => include_bytes!("../assets/icons/git-branch.svg"),
            Icon::Hexagon => include_bytes!("../assets/icons/hexagon.svg"),
            Icon::Info => include_bytes!("../assets/icons/info.svg"),
            Icon::Layers => include_bytes!("../assets/icons/layers.svg"),
            Icon::Lock => include_bytes!("../assets/icons/lock.svg"),
            Icon::Menu => include_bytes!("../assets/icons/menu.svg"),
            Icon::MicOff => include_bytes!("../assets/icons/mic-off.svg"),
            Icon::Mic => include_bytes!("../assets/icons/mic.svg"),
            Icon::Minus => include_bytes!("../assets/icons/minus.svg"),
            Icon::Monitor => include_bytes!("../assets/icons/monitor.svg"),
            Icon::Notebook => include_bytes!("../assets/icons/notebook.svg"),
            Icon::PanelRight => include_bytes!("../assets/icons/panel-right.svg"),
            Icon::Pin => include_bytes!("../assets/icons/pin.svg"),
            Icon::Play => include_bytes!("../assets/icons/play.svg"),
            Icon::Plus => include_bytes!("../assets/icons/plus.svg"),
            Icon::Refresh => include_bytes!("../assets/icons/refresh.svg"),
            Icon::Search => include_bytes!("../assets/icons/search.svg"),
            Icon::Send => include_bytes!("../assets/icons/send.svg"),
            Icon::Settings => include_bytes!("../assets/icons/settings.svg"),
            Icon::Shield => include_bytes!("../assets/icons/shield.svg"),
            Icon::Sidebar => include_bytes!("../assets/icons/sidebar.svg"),
            Icon::Smartphone => include_bytes!("../assets/icons/smartphone.svg"),
            Icon::Spinner => include_bytes!("../assets/icons/spinner.svg"),
            Icon::SquarePen => include_bytes!("../assets/icons/square-pen.svg"),
            Icon::Square => include_bytes!("../assets/icons/square.svg"),
            Icon::Star => include_bytes!("../assets/icons/star.svg"),
            Icon::Tasks => include_bytes!("../assets/icons/tasks.svg"),
            Icon::Terminal => include_bytes!("../assets/icons/terminal.svg"),
            Icon::Trash => include_bytes!("../assets/icons/trash.svg"),
            Icon::Triangle => include_bytes!("../assets/icons/triangle.svg"),
            Icon::Undo => include_bytes!("../assets/icons/undo.svg"),
            Icon::Unlock => include_bytes!("../assets/icons/unlock.svg"),
            Icon::Users => include_bytes!("../assets/icons/users.svg"),
            Icon::VoiceOff => include_bytes!("../assets/icons/voice-off.svg"),
            Icon::VoiceOn => include_bytes!("../assets/icons/voice-on.svg"),
            Icon::VolumeOff => include_bytes!("../assets/icons/volume-off.svg"),
            Icon::Volume => include_bytes!("../assets/icons/volume.svg"),
            Icon::X => include_bytes!("../assets/icons/x.svg"),
            Icon::Zap => include_bytes!("../assets/icons/zap.svg"),
        }
    }
}

pub fn icon(which: Icon) -> Svg {
    svg().path(which.path()).flex_none()
}

pub fn spinner(id: impl Into<ElementId>, size: f32, color: Hsla) -> impl IntoElement {
    icon(Icon::Spinner).size(px(size)).text_color(color).with_animation(
        id,
        Animation::new(Duration::from_millis(900)).repeat(),
        |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
    )
}

pub fn asset_bytes(path: &str) -> Option<&'static [u8]> {
    let name = path.strip_prefix("icons/")?.strip_suffix(".svg")?;
    crate::agent_icons::bytes(name).or_else(|| {
        Icon::ALL
            .iter()
            .find(|which| which.name() == name)
            .map(|which| which.bytes())
    })
}

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(asset_bytes(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(Icon::ALL
            .iter()
            .map(|which| which.path())
            .chain(crate::agent_icons::NAMES.iter().map(|name| {
                SharedString::from(format!("icons/{name}.svg"))
            }))
            .filter(|p| p.starts_with(path))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_loads_as_svg() {
        for path in Assets.list("icons/").unwrap() {
            let bytes = asset_bytes(&path).expect("icon bytes");
            let text = std::str::from_utf8(bytes).expect("utf8");
            assert!(text.contains("<svg"), "{path} is not an svg");
        }
    }

    #[test]
    fn unknown_paths_are_missing() {
        assert!(asset_bytes("icons/none.svg").is_none());
        assert!(asset_bytes("fonts/x.ttf").is_none());
        assert!(Assets.load("icons/nope.svg").unwrap().is_none());
    }

    #[test]
    fn list_finds_icons() {
        let all = Assets.list("icons/").unwrap();
        assert_eq!(all.len(), Icon::ALL.len() + crate::agent_icons::NAMES.len());
    }
}
