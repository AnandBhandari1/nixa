pub mod load;
pub mod model;
#[cfg(test)]
mod tests;
pub mod view;

pub use view::{BlameView, Open};

use super::host::Host;
use super::window;
use gpui::App;
use gpui::prelude::*;

pub const WIDTH: f32 = 1000.0;
pub const HEIGHT: f32 = 700.0;

pub fn open(open: Open, host: Host, cx: &mut App) {
    cx.defer(move |cx| {
        window::open_or_reuse::<BlameView>(
            window::Size {
                width: WIDTH,
                height: HEIGHT,
                title: "Who changed each line",
            },
            cx,
            {
                let open = open.clone();
                move |view, _, cx| view.reset(open.clone(), cx)
            },
            move |window, cx| cx.new(|cx| BlameView::new(open.clone(), host, window, cx)),
        );
    });
}
