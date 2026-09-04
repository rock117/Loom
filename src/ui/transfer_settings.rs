//! Pre-transfer settings form (dest path, include/exclude globs, compress).

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
    pub compress: bool,
    pub focus: TransferField,
}

impl TransferSettingsForm {
    pub fn for_download(entry: RemoteEntry, default_dest: String, last_compress: bool) -> Self {
        let mut dest = RenameEdit::new(default_dest);
        dest.select_all();
        Self {
            pending: PendingTransfer::Download { entry },
            dest,
            include: RenameEdit::new(""),
            exclude_extra: RenameEdit::new(""),
            exclude_presets: default_preset_checks(),
            compress: last_compress,
            focus: TransferField::Dest,
        }
    }

    pub fn for_upload(
        pane_id: Uuid,
        locals: Vec<PathBuf>,
        default_dest: String,
        last_compress: bool,
    ) -> Self {
        let mut dest = RenameEdit::new(default_dest);
        dest.select_all();
        Self {
            pending: PendingTransfer::Upload { pane_id, locals },
            dest,
            include: RenameEdit::new(""),
            exclude_extra: RenameEdit::new(""),
            exclude_presets: default_preset_checks(),
            compress: last_compress,
            focus: TransferField::Dest,
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
        self.focus = if reverse {
            match self.focus {
                TransferField::Dest => TransferField::ExcludeExtra,
                TransferField::Include => TransferField::Dest,
                TransferField::ExcludeExtra => TransferField::Include,
            }
        } else {
            match self.focus {
                TransferField::Dest => TransferField::Include,
                TransferField::Include => TransferField::ExcludeExtra,
                TransferField::ExcludeExtra => TransferField::Dest,
            }
        };
        self.focused_edit_mut().select_all();
    }

    pub fn title(&self) -> &'static str {
        match self.pending {
            PendingTransfer::Download { .. } => "Download settings",
            PendingTransfer::Upload { .. } => "Upload settings",
        }
    }

    pub fn dest_label(&self) -> &'static str {
        match self.pending {
            PendingTransfer::Download { .. } => "Local folder",
            PendingTransfer::Upload { .. } => "Remote directory",
        }
    }

    pub fn is_download(&self) -> bool {
        matches!(self.pending, PendingTransfer::Download { .. })
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
            compress: self.compress,
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
