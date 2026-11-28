use nixa_termview::Palette;
use nixa_ui::Theme;

pub fn palette(t: &Theme) -> Palette {
    Palette {
        background: t.terminal.background,
        foreground: t.terminal.foreground,
        selection: t.terminal.selection,
        cursor: t.terminal.cursor,
        ansi: t.terminal.ansi,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_theme_sets_the_terminal_colors() {
        let t = nixa_ui::theme::nixa_dark();
        let p = palette(&t);
        assert_eq!(p.background, t.terminal.background);
        assert_eq!(p.foreground, t.terminal.foreground);
        assert_eq!(p.selection, t.terminal.selection);
        assert_eq!(p.cursor, t.terminal.cursor);
        assert_eq!(p.ansi, t.terminal.ansi);
    }

    #[test]
    fn every_theme_gives_its_own_palette() {
        let list = nixa_ui::theme::builtin();
        assert!(list.len() > 1);
        let first = palette(&list[0]);
        assert!(list[1..].iter().any(|t| palette(t) != first));
    }
}
