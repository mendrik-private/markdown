use std::{env, path::PathBuf};

use anyhow::Result;
use mdtui_render::Theme;
use mdtui_terminal::TerminalGuard;
use mdtui_tui::{App, run};

fn main() -> Result<()> {
    let arg = env::args().nth(1);
    if matches!(arg.as_deref(), Some("-h" | "--help")) {
        println!(
            "mdtui-tui\n\nUsage:\n  mdtui-tui [path]\n\nKeys:\n  Ctrl-S save   Ctrl-Q quit   ? help   : command palette"
        );
        return Ok(());
    }
    let path = arg.map(PathBuf::from);
    let mut app = App::open_initial(path)?;
    let theme = Theme::ghostty_default_dark();
    let mut terminal = TerminalGuard::enter(&theme)?;
    let result = run(&mut terminal.terminal, &mut app);
    terminal.leave()?;
    result
}
