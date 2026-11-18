//! Workspace persistence and profile models.

pub mod forward;
pub mod persist;
pub mod profile;
pub mod snippets;
pub mod transfer_history;
pub mod workspace;

pub use forward::{
    PortForwardKind, PortForwardRule, format_open_ssh_command, open_ssh_forward_flag,
    suggest_free_bind_port,
};
pub use persist::{
    export_workspace_to, import_workspace_from, load_settings, load_snippets, load_transfer_history,
    load_ui_state, load_workspace, save_settings, save_snippets, save_transfer_history,
    save_ui_state, save_workspace,
};
pub use transfer_history::{
    PersistedTransfer, PersistedTransferDir, PersistedTransferStatus, TransferHistoryFile,
    scope_key_from_parts,
};
pub use profile::{
    ConnectionState, PaneSnapshot, Profile, ProfileKind, SavedPaneLayout, SavedSplitAxis, SshAuth,
    SshJumpHost,
};
pub use snippets::{Snippet, SnippetsFile};
pub use workspace::{
    AnsiPalette, Group, OpenTabRef, OrderKey, SettingsFile, SidebarEntry, SidebarProfileKind,
    UiStateFile, WorkspaceFile,
};
