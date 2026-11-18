//! Pre-transfer settings form (dest path, include/exclude globs).

use std::path::PathBuf;

use uuid::Uuid;

use crate::session::sftp::{RemoteEntry, TransferOptions};
use crate::session::transfer_filter::{
    DEFAULT_EXCLUDE_PRESETS, TransferFilter, parse_pattern_list,
};
use crate::ui::rename_edit::RenameEdit;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TransferField {
    Dest,
    Include,
    ExcludeExtra,
}

#[derive(Clone)]
pub enum PendingTransfer {
    Download {
        entry: RemoteEntry,
    },
    Upload {
        pane_id: Uuid,
        locals: Vec<PathBuf>,
    },
}

pub struct TransferSettingsForm {
    pub pending: PendingTransfer,
    pub dest: RenameEdit,
    pub include: RenameEdit,
    pub exclude_extra: RenameEdit,
    /// Preset exclude names with checkbox state (checked = excluded).
    pub exclude_presets: Vec<(String, bool)>,
    pub focus: TransferField,
    /// Show include / extra-exclude fields (collapsed by default).
    pub advanced_open: bool,
}

impl TransferSettingsForm {
    pub fn for_download(entry: RemoteEntry, default_dest: String) -> Self {
        let mut dest = RenameEdit::new(default_dest);
        dest.select_all();
        Self {
            pending: PendingTransfer::Download { entry },
            dest,
            include: RenameEdit::new(""),
            exclude_extra: RenameEdit::new(""),
            exclude_presets: default_preset_checks(),
            focus: TransferField::Dest,
            advanced_open: false,
        }
    }

    pub fn for_upload(pane_id: Uuid, locals: Vec<PathBuf>, default_dest: String) -> Self {
        let mut dest = RenameEdit::new(default_dest);
        dest.select_all();
        Self {
            pending: PendingTransfer::Upload { pane_id, locals },
            dest,
            include: RenameEdit::new(""),
            exclude_extra: RenameEdit::new(""),
            exclude_presets: default_preset_checks(),
            focus: TransferField::Dest,
            advanced_open: false,
        }
    }

    pub fn focused_edit_mut(&mut self) -> &mut RenameEdit {
        match self.focus {
            TransferField::Dest => &mut self.dest,
            TransferField::Include => &mut self.include,
            TransferField::ExcludeExtra => &mut self.exclude_extra,
        }
    }

    pub fn cycle_focus(&mut self, reverse: bool) {
        let fields = if self.advanced_open {
            [
                TransferField::Dest,
                TransferField::Include,
                TransferField::ExcludeExtra,
            ]
            .as_slice()
        } else {
            [TransferField::Dest].as_slice()
        };
        let idx = fields
            .iter()
            .position(|f| *f == self.focus)
            .unwrap_or(0);
        let next = if reverse {
            if idx == 0 {
                fields.len() - 1
            } else {
                idx - 1
            }
        } else {
            (idx + 1) % fields.len()
        };
        self.focus = fields[next];
        self.focused_edit_mut().select_all();
    }

    pub fn title(&self) -> &'static str {
        match self.pending {
            PendingTransfer::Download { .. } => "Download",
            PendingTransfer::Upload { .. } => "Upload",
        }
    }

    pub fn action_label(&self) -> &'static str {
        match self.pending {
            PendingTransfer::Download { .. } => "Download",
            PendingTransfer::Upload { .. } => "Upload",
        }
    }

    pub fn dest_label(&self) -> &'static str {
        match self.pending {
            PendingTransfer::Download { .. } => "Save to",
            PendingTransfer::Upload { .. } => "Remote folder",
        }
    }

    /// One-line summary of what will transfer (WinSCP / FinalShell style).
    pub fn summary(&self) -> String {
        match &self.pending {
            PendingTransfer::Download { entry } => {
                if entry.is_dir {
                    format!("Folder  ·  {}", entry.name)
                } else {
                    format!("File  ·  {}", entry.name)
                }
            }
            PendingTransfer::Upload { locals, .. } => match locals.as_slice() {
                [] => "Nothing selected".into(),
                [one] => {
                    let name = one
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| one.display().to_string());
                    if one.is_dir() {
                        format!("Folder  ·  {name}")
                    } else {
                        format!("File  ·  {name}")
                    }
                }
                many => {
                    let dirs = many.iter().filter(|p| p.is_dir()).count();
                    let files = many.len() - dirs;
                    match (dirs, files) {
                        (0, n) => format!("{n} files"),
                        (n, 0) => format!("{n} folders"),
                        (d, f) => format!("{d} folders, {f} files"),
                    }
                }
            },
        }
    }

    pub fn is_download(&self) -> bool {
        matches!(self.pending, PendingTransfer::Download { .. })
    }

    /// Whether exclude presets matter (folder / multi-item transfers).
    pub fn show_skip_presets(&self) -> bool {
        match &self.pending {
            PendingTransfer::Download { entry } => entry.is_dir,
            PendingTransfer::Upload { locals, .. } => {
                locals.len() > 1 || locals.iter().any(|p| p.is_dir())
            }
        }
    }

    pub fn build_options(&self) -> TransferOptions {
        let mut exclude: Vec<String> = self
            .exclude_presets
            .iter()
            .filter(|(_, on)| *on)
            .map(|(name, _)| name.clone())
            .collect();
        exclude.extend(parse_pattern_list(&self.exclude_extra.text));
        let include = parse_pattern_list(&self.include.text);
        TransferOptions {
            filter: TransferFilter::from_lists(include, exclude),
            compress: false,
        }
    }
}

fn default_preset_checks() -> Vec<(String, bool)> {
    DEFAULT_EXCLUDE_PRESETS
        .iter()
        .map(|s| ((*s).to_string(), true))
        .collect()
}

pub fn default_download_dir() -> String {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".".into())
}
