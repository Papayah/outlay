//! Movement: snap-move (swap, then alignment stops), nudge, exact moves, and spatial focus.

use super::geometry::{Dir, Rect};
use super::layout::{CommitReport, EditError, EditKind, Layout};
use super::links::Side;

/// How a snap-move went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapKind {
    /// Swapped places with this display along the move axis.
    Swapped(usize),
    /// Moved to the nearest alignment stop.
    Aligned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapped {
    pub kind: SnapKind,
    pub report: CommitReport,
}

impl Layout {
    /// Snap-move (`H J K L`). Tries, in order:
    /// 1. a swap with the display F touches on that side (longest shared edge), which reorders
    ///    a row: A|B|C with a move right on A gives B|A|C;
    /// 2. the nearest alignment stop in that direction where the moving set overlaps nothing
    ///    and F touches another display;
    /// 3. otherwise [`EditError::NoSnapSpot`], leaving the layout untouched.
    pub fn snap_move(&mut self, f: usize, dir: Dir) -> Result<Snapped, EditError> {
        self.check_movable(f)?;
        let before = self.clone();
        if let Some(n) = self.try_swap(f, dir) {
            let report = self.commit(&before, EditKind::Move);
            return Ok(Snapped {
                kind: SnapKind::Swapped(n),
                report,
            });
        }
        if self.try_alignment_stop(f, dir) {
            let report = self.commit(&before, EditKind::Move);
            return Ok(Snapped {
                kind: SnapKind::Aligned,
                report,
            });
        }
        Err(EditError::NoSnapSpot(dir))
    }

    fn check_movable(&self, f: usize) -> Result<(), EditError> {
        self.check_editable(f)?;
        if !self.is_enabled(f) {
            return Err(EditError::Refused(format!("{} is off.", self.names[f])));
        }
        if self.moving_set(f).iter().any(|&g| self.locked[g]) {
            return Err(EditError::Refused(format!(
                "{} would move a panning display.",
                self.names[f]
            )));
        }
        Ok(())
    }

    /// F's box and the displays stuck to it on the sides across the move axis: they travel with
    /// it in a swap.
    fn swap_group(&self, f: usize, horizontal: bool) -> Vec<usize> {
        let mirrors = self.mirror_group(f);
        let mut group = mirrors.clone();
        for &m in &mirrors {
            for c in self.children(m) {
                let side = self.links[c].expect("child has a link").side;
                let across = side != Side::Same && side.is_beside() != horizontal;
                if across {
                    for x in self.subtree(c) {
                        if !group.contains(&x) {
                            group.push(x);
                        }
                    }
                }
            }
        }
        group
    }

    /// Swaps F with its neighbour on the `dir` side, when that creates no overlap.
    fn try_swap(&mut self, f: usize, dir: Dir) -> Option<usize> {
        let f = self.mirror_root(f);
        let moving = self.moving_set(f);
        let fr = self.rect(f);
        let n = (0..self.len())
            .filter(|&o| self.is_enabled(o) && !moving.contains(&o) && !self.is_mirror_child(o))
            .filter_map(|o| {
                fr.shared_edge(&self.rect(o))
                    .filter(|&(d, _)| d == dir)
                    .map(|(_, len)| (o, len))
            })
            .max_by_key(|&(o, len)| (len, std::cmp::Reverse(o)))
            .map(|(o, _)| o)?;
        let nr = self.rect(n);
        let horizontal = dir.is_horizontal();
        let (fg, ng) = (
            self.swap_group(f, horizontal),
            self.swap_group(n, horizontal),
        );
        if fg.iter().any(|g| ng.contains(g)) || fg.iter().chain(&ng).any(|&g| self.locked[g]) {
            return None;
        }
        // Moving right: N takes F's place and F goes right after N. The pair keeps its span.
        let (df, dn) = match dir {
            Dir::Right => (nr.w, fr.x - nr.x),
            Dir::Left => (nr.x - fr.x, fr.w),
            Dir::Down => (nr.h, fr.y - nr.y),
            Dir::Up => (nr.y - fr.y, fr.h),
        };
        let delta = |d: i32| if horizontal { (d, 0) } else { (0, d) };
        let overlaps_before = self.overlapping_pairs();
        let mut trial = self.clone();
        let (fx, fy) = delta(df);
        let (nx, ny) = delta(dn);
        trial.translate(&fg, fx, fy);
        trial.translate(&ng, nx, ny);
        if trial
            .overlapping_pairs()
            .iter()
            .any(|p| !overlaps_before.contains(p))
        {
            return None;
        }
        *self = trial;
        Some(n)
    }

    /// Moves F's moving set to the nearest alignment stop in `dir`: F's start or end aligned
    /// with another display's start or end, or the centres aligned.
    fn try_alignment_stop(&mut self, f: usize, dir: Dir) -> bool {
        let f = self.mirror_root(f);
        let moving = self.moving_set(f);
        let outside: Vec<usize> = self
            .enabled()
            .into_iter()
            .filter(|o| !moving.contains(o))
            .collect();
        let fr = self.rect(f);
        let horizontal = dir.is_horizontal();
        let span = |r: &Rect| if horizontal { (r.x, r.w) } else { (r.y, r.h) };
        let (start, len) = span(&fr);
        let mut stops: Vec<i32> = outside
            .iter()
            .flat_map(|&o| {
                let (os, ol) = span(&self.rect(o));
                [
                    os,
                    os + ol,
                    os - len,
                    os + ol - len,
                    os + (ol - len).div_euclid(2),
                ]
            })
            .filter(|&c| (c - start) * dir.sign() > 0)
            .collect();
        stops.sort_by_key(|&c| (c - start).abs());
        stops.dedup();
        for stop in stops {
            let (dx, dy) = dir.delta((stop - start).abs());
            let mut trial = self.clone();
            trial.translate(&moving, dx, dy);
            let clear = !moving.iter().any(|&g| {
                outside
                    .iter()
                    .any(|&o| !trial.are_mirrors(g, o) && trial.rect(g).overlaps(&trial.rect(o)))
            });
            let touches = outside
                .iter()
                .any(|&o| trial.rect(f).shared_edge(&trial.rect(o)).is_some());
            if clear && touches {
                *self = trial;
                return true;
            }
        }
        false
    }

    /// Nudge (`Alt-h j k l`): moves F's moving set by exactly `step` pixels. Gaps and overlaps
    /// are allowed; validation flags them.
    pub fn nudge(&mut self, f: usize, dir: Dir, step: i32) -> Result<CommitReport, EditError> {
        let (dx, dy) = dir.delta(step);
        self.move_by(f, dx, dy)
    }

    /// `:move DX DY`
    pub fn move_by(&mut self, f: usize, dx: i32, dy: i32) -> Result<CommitReport, EditError> {
        self.check_movable(f)?;
        if dx == 0 && dy == 0 {
            return Err(EditError::NoChange("nothing to move".to_owned()));
        }
        let before = self.clone();
        let moving = self.moving_set(f);
        self.translate(&moving, dx, dy);
        Ok(self.commit(&before, EditKind::Move))
    }

    /// `:pos X Y`: moves F's moving set so F is at `x`,`y` before normalisation.
    pub fn move_to(&mut self, f: usize, x: i32, y: i32) -> Result<CommitReport, EditError> {
        let r = self.rect(self.mirror_root(f));
        self.move_by(f, x - r.x, y - r.y)
    }

    /// The nearest enabled display in `dir` from `f`: candidates have their centre in that
    /// half-plane, and the score is the main-axis distance plus twice the cross-axis offset.
    pub fn focus_towards(&self, f: usize, dir: Dir) -> Option<usize> {
        self.nearest_towards(f, dir, &[])
    }

    /// [`Layout::focus_towards`], never landing on a display in `skip`.
    pub fn nearest_towards(&self, f: usize, dir: Dir, skip: &[usize]) -> Option<usize> {
        if !self.is_enabled(f) {
            return None;
        }
        let root = self.mirror_root(f);
        let (fx, fy) = self.rect(root).center2();
        (0..self.len())
            .filter(|&o| {
                self.is_enabled(o)
                    && !self.is_mirror_child(o)
                    && !self.are_mirrors(o, root)
                    && !skip.contains(&o)
            })
            .filter_map(|o| {
                let (cx, cy) = self.rect(o).center2();
                let (main, cross) = match dir {
                    Dir::Right => (cx - fx, cy - fy),
                    Dir::Left => (fx - cx, cy - fy),
                    Dir::Down => (cy - fy, cx - fx),
                    Dir::Up => (fy - cy, cx - fx),
                };
                (main > 0).then_some((main + 2 * cross.abs(), o))
            })
            .min()
            .map(|(_, o)| o)
    }
}
