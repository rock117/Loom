use gpui::*;

use crate::shared::actions::*;
use crate::ui::workspace_view::WorkspaceView;

pub fn run() {
    let app = Application::new();

    app.run(|cx| {
        cx.bind_keys([
            KeyBinding::new("ctrl-t", NewLocalTab, Some("Loom")),
            KeyBinding::new("ctrl-w", CloseTab, Some("Loom")),
            KeyBinding::new("ctrl-tab", NextTab, Some("Loom")),
            KeyBinding::new("ctrl-shift-tab", PrevTab, Some("Loom")),
            KeyBinding::new("ctrl-shift-d", DuplicateTab, Some("Loom")),
            KeyBinding::new("f2", RenameFocused, Some("Loom")),
            KeyBinding::new("ctrl-s", SaveWorkspace, Some("Loom")),
            KeyBinding::new("ctrl-f", FocusSearch, Some("Loom")),
            KeyBinding::new("ctrl-,", ToggleSettings, Some("Loom")),
            KeyBinding::new("ctrl-e", ExportWorkspace, Some("Loom")),
            KeyBinding::new("ctrl-shift-i", ImportWorkspace, Some("Loom")),
            KeyBinding::new("ctrl-=", ZoomIn, Some("Loom")),
            KeyBinding::new("ctrl--", ZoomOut, Some("Loom")),
            KeyBinding::new("ctrl-0", ZoomReset, Some("Loom")),
        ]);

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
                |_window, cx| {
                    cx.new(|cx| WorkspaceView::new(_window, cx))
                },
            )?;
            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });
}
