pub mod io;
pub mod model;
#[cfg(test)]
mod tests;
pub mod view;

pub use view::{ConflictView, Open};

use super::host::Host;
use super::window;
use gpui::App;
use gpui::prelude::*;

pub const WIDTH: f32 = 980.0;
pub const HEIGHT: f32 = 680.0;

pub fn open(open: Open, host: Host, cx: &mut App) {
    cx.defer(move |cx| {
        window::open_or_reuse::<ConflictView>(
            window::Size {
                width: WIDTH,
                height: HEIGHT,
                title: "Resolve conflict",
            },
            cx,
            {
                let open = open.clone();
                move |view, _, cx| view.reset(open.clone(), cx)
            },
            move |window, cx| cx.new(|cx| ConflictView::new(open.clone(), host, window, cx)),
        );
    });
}
