//! Binary pane tree (Zed-style): split in four directions, activate by geometry.

use uuid::Uuid;

/// Side-by-side vs stacked children (matches Zed horizontal / vertical splits).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitAxis {
    /// Left | Right
    Horizontal,
    /// Top / Bottom
    Vertical,
}

/// Where the **new** pane is placed relative to the focused leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    Left,
    Right,
    Up,
    Down,
}

impl SplitDirection {
    pub fn axis(self) -> SplitAxis {
        match self {
            Self::Left | Self::Right => SplitAxis::Horizontal,
            Self::Up | Self::Down => SplitAxis::Vertical,
        }
    }

    /// If true, the new pane is the `first` child of the split.
    pub fn new_is_first(self) -> bool {
        matches!(self, Self::Left | Self::Up)
    }
}

/// Normalized frame inside the tab content (0..1).
#[derive(Clone, Copy, Debug)]
pub struct PaneFrame {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl PaneFrame {
    pub fn center(self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

#[derive(Clone, Debug)]
pub enum PaneLayout {
    Leaf(Uuid),
    Split {
        id: Uuid,
        axis: SplitAxis,
        /// Share of space for `first` (clamped on write).
        ratio: f32,
        first: Box<PaneLayout>,
        second: Box<PaneLayout>,
    },
}

impl PaneLayout {
    pub fn leaf(id: Uuid) -> Self {
        Self::Leaf(id)
    }

    pub fn clamp_ratio(ratio: f32) -> f32 {
        ratio.clamp(0.15, 0.85)
    }

    pub fn contains(&self, pane_id: Uuid) -> bool {
        match self {
            Self::Leaf(id) => *id == pane_id,
            Self::Split { first, second, .. } => {
                first.contains(pane_id) || second.contains(pane_id)
            }
        }
    }

    pub fn leaf_count(&self) -> usize {
        match self {
            Self::Leaf(_) => 1,
            Self::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    pub fn first_leaf(&self) -> Uuid {
        match self {
            Self::Leaf(id) => *id,
            Self::Split { first, .. } => first.first_leaf(),
        }
    }

    /// Replace `target` leaf with a split containing `target` and `new_id`.
    pub fn split(&mut self, target: Uuid, direction: SplitDirection, new_id: Uuid) -> bool {
        match self {
            Self::Leaf(id) if *id == target => {
                let old = Self::Leaf(target);
                let new_leaf = Self::Leaf(new_id);
                let (first, second) = if direction.new_is_first() {
                    (new_leaf, old)
                } else {
                    (old, new_leaf)
                };
                *self = Self::Split {
                    id: Uuid::new_v4(),
                    axis: direction.axis(),
                    ratio: 0.5,
                    first: Box::new(first),
                    second: Box::new(second),
                };
                true
            }
            Self::Leaf(_) => false,
            Self::Split { first, second, .. } => {
                first.split(target, direction, new_id) || second.split(target, direction, new_id)
            }
        }
    }

    /// Remove a leaf; collapses the parent split. Returns a sibling to focus, if any.
    pub fn remove_leaf(&mut self, pane_id: Uuid) -> RemoveResult {
        match self {
            Self::Leaf(id) => {
                if *id == pane_id {
                    RemoveResult::RemovedRoot
                } else {
                    RemoveResult::NotFound
                }
            }
            Self::Split { first, second, .. } => {
                if matches!(first.as_ref(), Self::Leaf(id) if *id == pane_id) {
                    let focus = second.first_leaf();
                    *self = std::mem::replace(second, Self::Leaf(Uuid::nil()));
                    return RemoveResult::Collapsed { focus };
                }
                if matches!(second.as_ref(), Self::Leaf(id) if *id == pane_id) {
                    let focus = first.first_leaf();
                    *self = std::mem::replace(first, Self::Leaf(Uuid::nil()));
                    return RemoveResult::Collapsed { focus };
                }
                match first.remove_leaf(pane_id) {
                    RemoveResult::NotFound => second.remove_leaf(pane_id),
                    other => other,
                }
            }
        }
    }

    pub fn set_ratio(&mut self, split_id: Uuid, ratio: f32) -> bool {
        match self {
            Self::Leaf(_) => false,
            Self::Split {
                id,
                ratio: r,
                first,
                second,
                ..
            } => {
                if *id == split_id {
                    *r = Self::clamp_ratio(ratio);
                    true
                } else {
                    first.set_ratio(split_id, ratio) || second.set_ratio(split_id, ratio)
                }
            }
        }
    }

    pub fn collect_frames(&self) -> Vec<(Uuid, PaneFrame)> {
        let mut out = Vec::new();
        self.collect_frames_into(
            PaneFrame {
                x: 0.0,
                y: 0.0,
                w: 1.0,
                h: 1.0,
            },
            &mut out,
        );
        out
    }

    fn collect_frames_into(&self, frame: PaneFrame, out: &mut Vec<(Uuid, PaneFrame)>) {
        match self {
            Self::Leaf(id) => out.push((*id, frame)),
            Self::Split {
                axis,
                ratio,
                first,
                second,
                ..
            } => {
                let ratio = Self::clamp_ratio(*ratio);
                match axis {
                    SplitAxis::Horizontal => {
                        let w1 = frame.w * ratio;
                        first.collect_frames_into(
                            PaneFrame {
                                x: frame.x,
                                y: frame.y,
                                w: w1,
                                h: frame.h,
                            },
                            out,
                        );
                        second.collect_frames_into(
                            PaneFrame {
                                x: frame.x + w1,
                                y: frame.y,
                                w: frame.w - w1,
                                h: frame.h,
                            },
                            out,
                        );
                    }
                    SplitAxis::Vertical => {
                        let h1 = frame.h * ratio;
                        first.collect_frames_into(
                            PaneFrame {
                                x: frame.x,
                                y: frame.y,
                                w: frame.w,
                                h: h1,
                            },
                            out,
                        );
                        second.collect_frames_into(
                            PaneFrame {
                                x: frame.x,
                                y: frame.y + h1,
                                w: frame.w,
                                h: frame.h - h1,
                            },
                            out,
                        );
                    }
                }
            }
        }
    }

    /// Zed `ActivatePaneInDirection`: nearest leaf whose center lies in that direction.
    pub fn adjacent_leaf(&self, focused: Uuid, direction: SplitDirection) -> Option<Uuid> {
        let frames = self.collect_frames();
        let (_, focus_frame) = frames.iter().find(|(id, _)| *id == focused)?;
        let (fx, fy) = focus_frame.center();

        let mut best: Option<(Uuid, f32)> = None;
        for (id, frame) in &frames {
            if *id == focused {
                continue;
            }
            let (cx, cy) = frame.center();
            let ok = match direction {
                SplitDirection::Left => cx < fx - 0.001,
                SplitDirection::Right => cx > fx + 0.001,
                SplitDirection::Up => cy < fy - 0.001,
                SplitDirection::Down => cy > fy + 0.001,
            };
            if !ok {
                continue;
            }
            let dist = (cx - fx).hypot(cy - fy);
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((*id, dist));
            }
        }
        best.map(|(id, _)| id)
    }
}

#[derive(Debug)]
pub enum RemoveResult {
    NotFound,
    RemovedRoot,
    Collapsed { focus: Uuid },
}
