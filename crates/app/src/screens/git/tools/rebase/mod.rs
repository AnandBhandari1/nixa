pub mod model;
pub mod plan;
#[cfg(test)]
mod tests;
pub mod view;

pub use view::{Open, RebaseView};

use super::host::Host;
use super::window;
use gpui::App;
use gpui::prelude::*;

pub const WIDTH: f32 = 820.0;
pub const HEIGHT: f32 = 640.0;

pub fn open(open: Open, host: Host, cx: &mut App) {
    cx.defer(move |cx| {
        window::open_or_reuse::<RebaseView>(
            window::Size {
                width: WIDTH,
                height: HEIGHT,
                title: "Change commits",
            },
            cx,
            {
                let open = open.clone();
                move |view, _, cx| view.reset(open.clone(), cx)
            },
            move |window, cx| cx.new(|cx| RebaseView::new(open.clone(), host, window, cx)),
        );
    });
}
