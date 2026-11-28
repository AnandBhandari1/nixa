use crate::workspace::Workspace;
use gpui::Context;
use gpui::prelude::*;

pub fn render(ws: &Workspace, cx: &mut Context<Workspace>) -> impl IntoElement + use<> {
    crate::screens::statusbar::host::host(ws, cx)
}
