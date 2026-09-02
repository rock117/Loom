//! Workspace persistence and profile models.

pub mod persist;
pub mod profile;
pub mod snippets;
pub mod workspace;

pub use persist::{
    export_workspace_to, import_workspace_from, load_settings, load_snippets, load_ui_state,
    load_workspace, save_settings, save_snippets, save_ui_state, save_workspace,
};
pub use profile::{ConnectionState, Profile, ProfileKind, SshAuth};
pub use snippets::{Snippet, SnippetsFile};
pub use workspace::{
    AnsiPalette, Group, OpenTabRef, OrderKey, SettingsFile, SidebarEntry, UiStateFile, WorkspaceFile,
};
