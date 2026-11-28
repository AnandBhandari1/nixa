use gpui::{App, KeyBinding, actions};

actions!(
    nixa_editor,
    [
        MoveLeft, MoveRight, MoveUp, MoveDown, SelectLeft, SelectRight, SelectUp, SelectDown, WordLeft, WordRight,
        SelectWordLeft, SelectWordRight, LineStart, LineEnd, SelectLineStart, SelectLineEnd, DocStart, DocEnd,
        SelectDocStart, SelectDocEnd, PageUp, PageDown, SelectPageUp, SelectPageDown, Backspace, Delete,
        DeleteWordBack, DeleteLineBack, Newline, Tab, Outdent, SelectAll, Copy, Cut, Paste, Undo, Redo, Save, Find,
        FindNext, FindPrev, Escape
    ]
);

pub const ROOT_CONTEXT: &str = "NixaEditor";
pub const TEXT_CONTEXT: &str = "NixaEditorText";
pub const PREVIEW_CONTEXT: &str = "NixaEditorPreview";

pub fn bind_keys(cx: &mut App) {
    let root = Some(ROOT_CONTEXT);
    let text = Some(TEXT_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("cmd-s", Save, root),
        KeyBinding::new("cmd-f", Find, root),
        KeyBinding::new("cmd-g", FindNext, root),
        KeyBinding::new("cmd-shift-g", FindPrev, root),
        KeyBinding::new("escape", Escape, root),
        KeyBinding::new("left", MoveLeft, text),
        KeyBinding::new("right", MoveRight, text),
        KeyBinding::new("up", MoveUp, text),
        KeyBinding::new("down", MoveDown, text),
        KeyBinding::new("shift-left", SelectLeft, text),
        KeyBinding::new("shift-right", SelectRight, text),
        KeyBinding::new("shift-up", SelectUp, text),
        KeyBinding::new("shift-down", SelectDown, text),
        KeyBinding::new("alt-left", WordLeft, text),
        KeyBinding::new("alt-right", WordRight, text),
        KeyBinding::new("alt-shift-left", SelectWordLeft, text),
        KeyBinding::new("alt-shift-right", SelectWordRight, text),
        KeyBinding::new("cmd-left", LineStart, text),
        KeyBinding::new("home", LineStart, text),
        KeyBinding::new("cmd-right", LineEnd, text),
        KeyBinding::new("end", LineEnd, text),
        KeyBinding::new("cmd-shift-left", SelectLineStart, text),
        KeyBinding::new("shift-home", SelectLineStart, text),
        KeyBinding::new("cmd-shift-right", SelectLineEnd, text),
        KeyBinding::new("shift-end", SelectLineEnd, text),
        KeyBinding::new("cmd-up", DocStart, text),
        KeyBinding::new("cmd-down", DocEnd, text),
        KeyBinding::new("cmd-shift-up", SelectDocStart, text),
        KeyBinding::new("cmd-shift-down", SelectDocEnd, text),
        KeyBinding::new("pageup", PageUp, text),
        KeyBinding::new("pagedown", PageDown, text),
        KeyBinding::new("shift-pageup", SelectPageUp, text),
        KeyBinding::new("shift-pagedown", SelectPageDown, text),
        KeyBinding::new("backspace", Backspace, text),
        KeyBinding::new("shift-backspace", Backspace, text),
        KeyBinding::new("delete", Delete, text),
        KeyBinding::new("alt-backspace", DeleteWordBack, text),
        KeyBinding::new("cmd-backspace", DeleteLineBack, text),
        KeyBinding::new("enter", Newline, text),
        KeyBinding::new("tab", Tab, text),
        KeyBinding::new("shift-tab", Outdent, text),
        KeyBinding::new("cmd-a", SelectAll, text),
        KeyBinding::new("cmd-c", Copy, text),
        KeyBinding::new("cmd-a", SelectAll, Some(PREVIEW_CONTEXT)),
        KeyBinding::new("cmd-c", Copy, Some(PREVIEW_CONTEXT)),
        KeyBinding::new("cmd-x", Cut, text),
        KeyBinding::new("cmd-v", Paste, text),
        KeyBinding::new("cmd-z", Undo, text),
        KeyBinding::new("cmd-shift-z", Redo, text),
    ]);
}
