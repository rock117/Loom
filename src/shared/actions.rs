use gpui::*;

actions!(
    loom,
    [
        NewLocalTab,
        CloseTab,
        NextTab,
        PrevTab,
        DuplicateTab,
        RenameFocused,
        SaveWorkspace,
        FocusSearch,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ToggleSettings,
        ExportWorkspace,
        ImportWorkspace,
    ]
);
