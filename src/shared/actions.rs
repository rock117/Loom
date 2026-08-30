use gpui::*;

actions!(
    loom,
    [
        /// Open a new local shell tab from the default profile.
        NewLocalTab,
        /// Close the active tab.
        CloseTab,
        /// Activate the next tab.
        NextTab,
        /// Activate the previous tab.
        PrevTab,
        /// Duplicate the active tab session.
        DuplicateTab,
        /// Rename the selected sidebar profile or group (F2 in Renamable context).
        RenameFocused,
        /// Persist workspace and UI state immediately.
        SaveWorkspace,
        /// Increase terminal font size.
        ZoomIn,
        /// Decrease terminal font size.
        ZoomOut,
        /// Reset terminal font size.
        ZoomReset,
        /// Toggle the settings overlay.
        ToggleSettings,
        /// Show or hide the profiles sidebar (Zed project-panel toggle).
        ToggleSidebar,
        /// Show or hide the right context panel (snippets / files / info).
        ToggleContextPanel,
        /// Export workspace JSON via file dialog.
        ExportWorkspace,
        /// Import workspace JSON (replaces current).
        ImportWorkspace,
        /// Quit the application.
        QuitApp,
        /// Split the focused pane to the left (Zed `pane::SplitLeft`).
        SplitLeft,
        /// Split the focused pane to the right (Zed `pane::SplitRight`).
        SplitRight,
        /// Split the focused pane upward (Zed `pane::SplitUp`).
        SplitUp,
        /// Split the focused pane downward (Zed `pane::SplitDown`).
        SplitDown,
        /// Focus the pane to the left (Zed `workspace::ActivatePaneLeft`).
        ActivatePaneLeft,
        /// Focus the pane to the right.
        ActivatePaneRight,
        /// Focus the pane above.
        ActivatePaneUp,
        /// Focus the pane below.
        ActivatePaneDown,
    ]
);
