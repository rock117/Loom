//! Zed-inspired visual tokens for Loom (original values; not copied from Zed).

use gpui::Hsla;

// --- Surfaces (dark, layered) ---

pub const BG: Hsla = Hsla {
    h: 0.60,
    s: 0.04,
    l: 0.11,
    a: 1.0,
};
pub const SIDEBAR_BG: Hsla = Hsla {
    h: 0.60,
    s: 0.05,
    l: 0.09,
    a: 1.0,
};
pub const PANEL_BG: Hsla = Hsla {
    h: 0.60,
    s: 0.04,
    l: 0.13,
    a: 1.0,
};
pub const ELEVATED: Hsla = Hsla {
    h: 0.60,
    s: 0.04,
    l: 0.16,
    a: 1.0,
};
pub const BORDER: Hsla = Hsla {
    h: 0.60,
    s: 0.04,
    l: 0.20,
    a: 1.0,
};
pub const BORDER_SUBTLE: Hsla = Hsla {
    h: 0.60,
    s: 0.03,
    l: 0.17,
    a: 1.0,
};

// --- Text ---

pub const TEXT: Hsla = Hsla {
    h: 0.55,
    s: 0.04,
    l: 0.90,
    a: 1.0,
};
pub const TEXT_MUTED: Hsla = Hsla {
    h: 0.55,
    s: 0.04,
    l: 0.58,
    a: 1.0,
};
pub const TEXT_DISABLED: Hsla = Hsla {
    h: 0.55,
    s: 0.03,
    l: 0.40,
    a: 1.0,
};

// --- Accents / state ---

pub const ACCENT: Hsla = Hsla {
    h: 0.58,
    s: 0.45,
    l: 0.55,
    a: 1.0,
};
pub const ACCENT_SOFT: Hsla = Hsla {
    h: 0.58,
    s: 0.30,
    l: 0.22,
    a: 1.0,
};
pub const DANGER: Hsla = Hsla {
    h: 0.02,
    s: 0.60,
    l: 0.52,
    a: 1.0,
};
pub const SUCCESS: Hsla = Hsla {
    h: 0.35,
    s: 0.45,
    l: 0.45,
    a: 1.0,
};
pub const TAB_ACTIVE: Hsla = Hsla {
    h: 0.60,
    s: 0.04,
    l: 0.17,
    a: 1.0,
};
pub const HOVER: Hsla = Hsla {
    h: 0.60,
    s: 0.04,
    l: 0.18,
    a: 1.0,
};
pub const SELECTION: Hsla = Hsla {
    h: 0.58,
    s: 0.25,
    l: 0.24,
    a: 1.0,
};

/// Local shell / terminal glyph in the sidebar.
pub const ICON_LOCAL: Hsla = Hsla {
    h: 0.48,
    s: 0.42,
    l: 0.55,
    a: 1.0,
};
/// SSH / remote glyph in the sidebar.
pub const ICON_REMOTE: Hsla = Hsla {
    h: 0.62,
    s: 0.40,
    l: 0.58,
    a: 1.0,
};
/// Group / folder glyph.
pub const ICON_GROUP: Hsla = Hsla {
    h: 0.12,
    s: 0.35,
    l: 0.58,
    a: 1.0,
};

// --- Type / spacing (logical px) ---

pub const FONT_UI: f32 = 13.0;
pub const FONT_UI_SM: f32 = 12.0;
pub const FONT_UI_LG: f32 = 14.0;
pub const FONT_TERM_DEFAULT: f32 = 14.0;

pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;

pub const SIDEBAR_MIN: f32 = 180.0;
pub const SIDEBAR_MAX: f32 = 480.0;
pub const RADIUS: f32 = 6.0;
pub const RADIUS_SM: f32 = 4.0;
pub const TAB_BAR_HEIGHT: f32 = 36.0;
pub const STATUS_BAR_PAD_X: f32 = 12.0;
pub const STATUS_BAR_PAD_Y: f32 = 4.0;
