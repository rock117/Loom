use gpui::*;

use crate::assets::Assets;
use crate::shared::actions::*;
use crate::ui::workspace_view::WorkspaceView;

pub fn run() {
    let app = Application::new().with_assets(Assets);

    app.run(|cx| {
        cx.bind_keys([
            KeyBinding::new("ctrl-t", NewLocalTab, Some("Loom")),
            KeyBinding::new("ctrl-w", CloseTab, Some("Loom")),
            KeyBinding::new("ctrl-tab", NextTab, Some("Loom")),
            KeyBinding::new("ctrl-shift-tab", PrevTab, Some("Loom")),
            KeyBinding::new("ctrl-shift-d", DuplicateTab, Some("Loom")),
            KeyBinding::new("f2", RenameFocused, Some("Renamable")),
            KeyBinding::new("ctrl-s", SaveWorkspace, Some("Loom")),
            KeyBinding::new("ctrl-,", ToggleSettings, Some("Loom")),
            KeyBinding::new("ctrl-b", ToggleSidebar, Some("Loom")),
            KeyBinding::new("ctrl-shift-b", ToggleContextPanel, Some("Loom")),
            KeyBinding::new("ctrl-e", ExportWorkspace, Some("Loom")),
            KeyBinding::new("ctrl-shift-i", ImportWorkspace, Some("Loom")),
            KeyBinding::new("ctrl-=", ZoomIn, Some("Loom")),
            KeyBinding::new("ctrl--", ZoomOut, Some("Loom")),
            KeyBinding::new("ctrl-0", ZoomReset, Some("Loom")),
            // Ctrl+Q quits; leave Ctrl+C for the shell.
            KeyBinding::new("ctrl-q", QuitApp, Some("Loom")),
            // Zed-style pane splits (ctrl-k chord) + VS Code-ish SplitRight.
            KeyBinding::new("ctrl-k left", SplitLeft, Some("Loom")),
            KeyBinding::new("ctrl-k right", SplitRight, Some("Loom")),
            KeyBinding::new("ctrl-k up", SplitUp, Some("Loom")),
            KeyBinding::new("ctrl-k down", SplitDown, Some("Loom")),
            KeyBinding::new("ctrl-\\", SplitRight, Some("Loom")),
            KeyBinding::new("ctrl-k ctrl-left", ActivatePaneLeft, Some("Loom")),
            KeyBinding::new("ctrl-k ctrl-right", ActivatePaneRight, Some("Loom")),
            KeyBinding::new("ctrl-k ctrl-up", ActivatePaneUp, Some("Loom")),
            KeyBinding::new("ctrl-k ctrl-down", ActivatePaneDown, Some("Loom")),
        ]);

        cx.on_action(|_: &QuitApp, cx| {
            cx.quit();
        });

        // When the last window closes, exit the process (GPUI does not auto-quit).
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        cx.spawn(async move |cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(80.0), px(60.0)),
                        size: size(px(1280.0), px(800.0)),
                    })),
                    titlebar: Some(TitlebarOptions {
                        title: Some("Loom".into()),
                        appears_transparent: false,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_window, cx| cx.new(|cx| WorkspaceView::new(_window, cx)),
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
