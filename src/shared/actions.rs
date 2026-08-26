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
        /// Rename the focused profile, group, or tab.
        RenameFocused,
        /// Persist workspace and UI state immediately.
        SaveWorkspace,
        /// Focus the sidebar search field.
        FocusSearch,
        /// Increase terminal font size.
        ZoomIn,
        /// Decrease terminal font size.
        ZoomOut,
        /// Reset terminal font size.
        ZoomReset,
        /// Toggle the settings overlay.
        ToggleSettings,
        /// Export workspace JSON via file dialog.
        ExportWorkspace,
        /// Import workspace JSON (replaces current).
        ImportWorkspace,
    ]
);
