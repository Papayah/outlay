//! Persistent stick links: a directed forest where each child is placed from its parent.
//!
//! A display without a link is an anchor and its stored position is authoritative. Links are
//! inferred from touching edges when a layout is loaded, follow positions after a move
//! (`relink`), and place displays after anything else (`resolve`).

use std::cmp::Reverse;
use std::collections::VecDeque;

use super::Snapshot;
use super::geometry::{Dir, Point, Rect, Size};
use super::layout::{CommitReport, EditError, EditKind, Layout};

/// Where a child sits relative to its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    LeftOf,
    RightOf,
    Above,
    Below,
    /// A mirror: the child has the parent's position.
    Same,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::LeftOf => Side::RightOf,
            Side::RightOf => Side::LeftOf,
            Side::Above => Side::Below,
            Side::Below => Side::Above,
            Side::Same => Side::Same,
        }
    }

    /// The side a child is on when it touches its parent's `dir` edge.
    pub fn from_dir(dir: Dir) -> Side {
        match dir {
            Dir::Left => Side::LeftOf,
            Dir::Right => Side::RightOf,
            Dir::Up => Side::Above,
            Dir::Down => Side::Below,
        }
    }

    /// Left-of and right-of share a vertical edge, so alignment runs along y.
    pub fn is_beside(self) -> bool {
        matches!(self, Side::LeftOf | Side::RightOf)
    }

    /// The xrandr option word.
    pub fn as_str(self) -> &'static str {
        match self {
            Side::LeftOf => "left-of",
            Side::RightOf => "right-of",
            Side::Above => "above",
            Side::Below => "below",
            Side::Same => "same-as",
        }
    }
}

/// Where the child sits along the shared edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Align {
    Start,
    Center,
    End,
}

impl Align {
    pub const ALL: [Align; 3] = [Align::Start, Align::Center, Align::End];

    /// The alignment a new stick starts with: `Start` beside (xrandr's `--right-of` behaviour),
    /// `Center` above and below.
    pub fn default_for(side: Side) -> Align {
        if side.is_beside() {
            Align::Start
        } else {
            Align::Center
        }
    }

    /// `top`/`middle`/`bottom` beside, `left`/`centre`/`right` above and below.
    pub fn label(self, side: Side) -> &'static str {
        match (side.is_beside(), self) {
            (true, Align::Start) => "top",
            (true, Align::Center) => "middle",
            (true, Align::End) => "bottom",
            (false, Align::Start) => "left",
            (false, Align::Center) => "centre",
            (false, Align::End) => "right",
        }
    }

    pub fn next(self) -> Align {
        match self {
            Align::Start => Align::Center,
            Align::Center => Align::End,
            Align::End => Align::Start,
        }
    }

    pub fn previous(self) -> Align {
        match self {
            Align::Start => Align::End,
            Align::Center => Align::Start,
            Align::End => Align::Center,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    pub parent: usize,
    pub side: Side,
    /// Unused for `Same`.
    pub align: Align,
    /// Pixels along the shared edge, added to the aligned position. Unused for `Same`.
    pub offset: i32,
}

/// What turning a display off changed, so turning it back on can undo it.
#[derive(Clone, Debug, PartialEq)]
pub struct Restore {
    /// The display that inherited the links, or any other display when there was nothing to
    /// inherit; the display comes back at `reference + offset`.
    pub reference: usize,
    pub offset: Point,
    /// The display's own link.
    pub link: Option<Link>,
    /// Each former child: its link to the display, and the link turning off gave it instead.
    pub children: Vec<(usize, Link, Option<Link>)>,
    pub primary: bool,
}

/// Where a child of length `clen` starts along a parent edge starting at `start` of length `plen`.
pub fn anchor(start: i32, plen: i32, clen: i32, align: Align) -> i32 {
    match align {
        Align::Start => start,
        Align::Center => start + (plen - clen).div_euclid(2),
        Align::End => start + plen - clen,
    }
}

/// The position of a child of `size` stuck to `parent`. The offset is clamped so the shared edge
/// stays at least one pixel long: a link never shrinks to a corner touch.
pub fn place(size: Size, parent: Rect, link: &Link) -> Point {
    let along = |p0: i32, plen: i32, clen: i32| {
        (anchor(p0, plen, clen, link.align) + link.offset).clamp(p0 - clen + 1, p0 + plen - 1)
    };
    match link.side {
        Side::Same => parent.pos(),
        Side::RightOf => Point::new(parent.right(), along(parent.y, parent.h, size.h)),
        Side::LeftOf => Point::new(parent.x - size.w, along(parent.y, parent.h, size.h)),
        Side::Below => Point::new(along(parent.x, parent.w, size.w), parent.bottom()),
        Side::Above => Point::new(along(parent.x, parent.w, size.w), parent.y - size.h),
    }
}

/// The alignment rule: the alignment with the smallest |offset| for `child` on `side` of
/// `parent`. Ties go to `Center` above and below and to `Start` beside, so a laptop nudged off
/// centre under a monitor becomes `Center+10` and stays centred when the monitor changes mode.
pub fn best_align(side: Side, child: Rect, parent: Rect) -> (Align, i32) {
    let (c0, clen, p0, plen) = if side.is_beside() {
        (child.y, child.h, parent.y, parent.h)
    } else {
        (child.x, child.w, parent.x, parent.w)
    };
    let order = match side {
        Side::Same => return (Align::Start, 0),
        Side::LeftOf | Side::RightOf => [Align::Start, Align::Center, Align::End],
        Side::Above | Side::Below => [Align::Center, Align::Start, Align::End],
    };
    // `min_by_key` keeps the first of equal keys, so the order above breaks ties.
    order
        .into_iter()
        .map(|a| (a, c0 - anchor(p0, plen, clen, a)))
        .min_by_key(|(_, off)| off.abs())
        .expect("three alignments")
}

impl Layout {
    fn link_to(&self, child: usize, parent: usize, side: Side) -> Link {
        let (align, offset) = best_align(side, self.rect(child), self.rect(parent));
        Link {
            parent,
            side,
            align,
            offset,
        }
    }

    /// The parent a display is placed from, ignoring links to displays that are off.
    fn parent_of(&self, i: usize) -> Option<usize> {
        self.links[i]
            .map(|l| l.parent)
            .filter(|&p| self.is_enabled(p))
    }

    /// Enabled children of `p`, in index order.
    pub fn children(&self, p: usize) -> Vec<usize> {
        (0..self.len())
            .filter(|&c| self.is_enabled(c) && self.links[c].is_some_and(|l| l.parent == p))
            .collect()
    }

    /// `i` and every enabled display stuck to it, directly or not, in breadth-first order.
    pub fn subtree(&self, i: usize) -> Vec<usize> {
        let mut out = vec![i];
        let mut k = 0;
        while k < out.len() {
            for c in self.children(out[k]) {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
            k += 1;
        }
        out
    }

    pub fn is_mirror_child(&self, i: usize) -> bool {
        self.links[i].is_some_and(|l| l.side == Side::Same) && self.parent_of(i).is_some()
    }

    /// The display a mirror follows, through any chain of `Same` links.
    pub fn mirror_root(&self, mut i: usize) -> usize {
        for _ in 0..self.len() {
            if !self.is_mirror_child(i) {
                break;
            }
            i = self.links[i].expect("mirror child has a link").parent;
        }
        i
    }

    /// `root` and every display mirroring it.
    pub fn mirror_group(&self, root: usize) -> Vec<usize> {
        let mut out = vec![root];
        let mut k = 0;
        while k < out.len() {
            for c in self.children(out[k]) {
                if self.links[c].is_some_and(|l| l.side == Side::Same) && !out.contains(&c) {
                    out.push(c);
                }
            }
            k += 1;
        }
        out
    }

    pub fn are_mirrors(&self, a: usize, b: usize) -> bool {
        self.mirror_root(a) == self.mirror_root(b)
    }

    /// The moving set G(F): F's box (its mirror group) and everything stuck to it. When that
    /// would be every enabled display, F moves alone with its mirrors, and `relink()` re-attaches
    /// the children it leaves behind.
    pub fn moving_set(&self, f: usize) -> Vec<usize> {
        let root = self.mirror_root(f);
        let group = self.subtree(root);
        if group.len() >= self.enabled_count() {
            self.mirror_group(root)
        } else {
            group
        }
    }

    /// Rebuilds every link from the current positions:
    /// 1. displays with identical rectangles become `Same` children of the primary among them,
    ///    else of the lowest index;
    /// 2. the rest form components of displays whose edges touch;
    /// 3. each component is walked breadth-first from its root (the primary, else the largest,
    ///    then the lowest index), visiting neighbours by longest shared edge, then index.
    pub fn infer_links(&mut self) {
        let n = self.len();
        self.links = vec![None; n];
        let on = self.enabled();
        let rects: Vec<Rect> = (0..n).map(|i| self.rect(i)).collect();
        let prefer: Vec<(bool, bool)> = (0..n)
            .map(|i| (self.locked[i], self.outputs[i].primary))
            .collect();
        let rank: Vec<_> = (0..n)
            .map(|i| (prefer[i], rects[i].area(), Reverse(i)))
            .collect();

        let mut reps: Vec<usize> = Vec::new();
        for &i in &on {
            match reps.iter().position(|&r| rects[r] == rects[i]) {
                None => reps.push(i),
                Some(k) => {
                    // A locked or primary display is the one others mirror.
                    let r = reps[k];
                    let (root, child) = if prefer[i] > prefer[r] {
                        (i, r)
                    } else {
                        (r, i)
                    };
                    reps[k] = root;
                    for c in 0..n {
                        if c == child
                            || self.links[c]
                                .is_some_and(|l| l.parent == child && l.side == Side::Same)
                        {
                            self.links[c] = Some(Link {
                                parent: root,
                                side: Side::Same,
                                align: Align::Start,
                                offset: 0,
                            });
                        }
                    }
                }
            }
        }

        let mut visited = vec![false; n];
        loop {
            let Some(root) = reps
                .iter()
                .copied()
                .filter(|&r| !visited[r])
                .max_by_key(|&r| rank[r])
            else {
                break;
            };
            visited[root] = true;
            let mut queue = VecDeque::from([root]);
            while let Some(u) = queue.pop_front() {
                let mut next: Vec<(usize, Side, i32)> = reps
                    .iter()
                    .copied()
                    .filter(|&v| !visited[v] && !self.locked[v])
                    .filter_map(|v| {
                        rects[u]
                            .shared_edge(&rects[v])
                            .map(|(dir, len)| (v, Side::from_dir(dir), len))
                    })
                    .collect();
                next.sort_by_key(|&(v, _, len)| (Reverse(len), v));
                for (v, side, _) in next {
                    visited[v] = true;
                    self.links[v] = Some(self.link_to(v, u, side));
                    queue.push_back(v);
                }
            }
        }
    }

    /// Places every stuck display from its parent, breadth-first from each anchor.
    pub fn resolve(&mut self) {
        let n = self.len();
        let mut queue: VecDeque<usize> = (0..n)
            .filter(|&i| self.is_enabled(i) && self.parent_of(i).is_none())
            .collect();
        let mut seen = vec![false; n];
        while let Some(u) = queue.pop_front() {
            if std::mem::replace(&mut seen[u], true) {
                continue;
            }
            for c in self.children(u) {
                let link = self.links[c].expect("child has a link");
                self.outputs[c].pos = place(self.size(c), self.rect(u), &link);
                queue.push_back(c);
            }
        }
    }

    /// Makes links follow positions after a move:
    /// - a child still touching its parent on the recorded side (or still at the same position,
    ///   for a mirror) keeps the side and gets align and offset from the alignment rule;
    /// - every other link is detached, and the detached displays attach, in index order, to the
    ///   touching display with the longest shared edge outside their own subtree, repeating
    ///   until a pass attaches nothing;
    /// - anchors stay anchors, so an unstick is respected.
    pub(super) fn relink(&mut self) {
        let mut detached = Vec::new();
        for c in 0..self.len() {
            let Some(link) = self.links[c] else { continue };
            let (child, parent) = (self.rect(c), self.rect(link.parent));
            let keeps = self.is_enabled(c)
                && self.is_enabled(link.parent)
                && match link.side {
                    Side::Same => child.pos() == parent.pos(),
                    side => parent
                        .shared_edge(&child)
                        .is_some_and(|(dir, _)| Side::from_dir(dir) == side),
                };
            if !keeps {
                self.links[c] = None;
                if self.is_enabled(c) {
                    detached.push(c);
                }
            } else if link.side != Side::Same {
                self.links[c] = Some(self.link_to(c, link.parent, link.side));
            }
        }
        loop {
            let mut attached = false;
            for &c in &detached {
                if self.links[c].is_some() {
                    continue;
                }
                if let Some((t, side)) = self.best_touching(c) {
                    self.links[c] = Some(self.link_to(c, t, side));
                    attached = true;
                }
            }
            if !attached {
                break;
            }
        }
    }

    /// The display `c` touches along the longest edge, outside `c`'s own subtree.
    fn best_touching(&self, c: usize) -> Option<(usize, Side)> {
        let own = self.subtree(c);
        (0..self.len())
            .filter(|&t| self.is_enabled(t) && !own.contains(&t) && !self.is_mirror_child(t))
            .filter_map(|t| {
                self.rect(t)
                    .shared_edge(&self.rect(c))
                    .map(|(dir, len)| (t, Side::from_dir(dir), len))
            })
            .max_by_key(|&(t, _, len)| (len, Reverse(t)))
            .map(|(t, side, _)| (t, side))
    }

    /// Handles overlaps a resize created. For each pair that touched in `before` and overlaps
    /// now, where only one of the two moved or grew, the other one's moving set is pushed
    /// outward across the old seam by the overlap depth. Each display is pushed at most once;
    /// a push that would create a new overlap is undone and reported. Returns whether anything
    /// was pushed.
    pub(super) fn push_pass(&mut self, before: &Layout, report: &mut CommitReport) -> bool {
        let n = self.len();
        let both = |l: &Layout, i: usize| before.is_enabled(i) && l.is_enabled(i);
        let changed: Vec<bool> = (0..n)
            .map(|i| both(self, i) && before.rect(i) != self.rect(i))
            .collect();
        let mut pushed = vec![false; n];
        let mut any = false;
        for i in 0..n {
            for j in i + 1..n {
                if !both(self, i) || !both(self, j) || self.are_mirrors(i, j) {
                    continue;
                }
                let Some((seam, _)) = before.rect(i).shared_edge(&before.rect(j)) else {
                    continue;
                };
                if !self.rect(i).overlaps(&self.rect(j)) {
                    continue;
                }
                // `dir` is the side of `mover` the other display sat on.
                let (mover, other, dir) = match (changed[i], changed[j]) {
                    (true, false) => (i, j, seam),
                    (false, true) => (j, i, seam.opposite()),
                    _ => continue,
                };
                let (m, o) = (self.rect(mover), self.rect(other));
                let depth = match dir {
                    Dir::Right => m.right() - o.x,
                    Dir::Left => o.right() - m.x,
                    Dir::Down => m.bottom() - o.y,
                    Dir::Up => o.bottom() - m.y,
                };
                if depth <= 0 {
                    continue;
                }
                let mut group = self.moving_set(other);
                if group.contains(&mover) {
                    group = self.mirror_group(self.mirror_root(other));
                }
                let blocked = format!(
                    "Could not push {} clear of {}.",
                    self.names[other], self.names[mover]
                );
                if group.contains(&mover) || group.iter().any(|&g| pushed[g] || self.locked[g]) {
                    report.notes.push(blocked);
                    continue;
                }
                let overlaps_before = self.overlapping_pairs();
                let (dx, dy) = dir.delta(depth);
                self.translate(&group, dx, dy);
                if self
                    .overlapping_pairs()
                    .iter()
                    .any(|p| !overlaps_before.contains(p))
                {
                    self.translate(&group, -dx, -dy);
                    report.notes.push(blocked);
                    continue;
                }
                for &g in &group {
                    pushed[g] = true;
                }
                report.pushed.extend(&group);
                any = true;
            }
        }
        any
    }

    pub(super) fn translate(&mut self, group: &[usize], dx: i32, dy: i32) {
        for &g in group {
            let p = self.outputs[g].pos;
            self.outputs[g].pos = Point::new(p.x + dx, p.y + dy);
        }
    }

    /// Makes `t` the root of its tree: every link on the path from `t` to the old root is
    /// reversed, with align and offset recomputed from positions. A locked display never becomes
    /// a child, so the path stops below one; nothing moves either way.
    fn reroot(&mut self, t: usize) {
        let mut path = vec![t];
        while let Some(p) = self.parent_of(*path.last().expect("path is not empty")) {
            if path.contains(&p) || self.locked[p] {
                break;
            }
            path.push(p);
        }
        let old: Vec<Option<Link>> = path.iter().map(|&p| self.links[p]).collect();
        for k in 0..path.len() - 1 {
            let (child, parent) = (path[k], path[k + 1]);
            let side = old[k].expect("path link").side.opposite();
            self.links[parent] = Some(self.link_to(parent, child, side));
        }
        self.links[t] = None;
    }

    /// Sticks `f` to `side` of `t`:
    /// 1. if `t` is in `f`'s subtree, `f`'s child on the path to `t` becomes an anchor;
    /// 2. `t`'s tree is re-rooted at `t`;
    /// 3. `f` links to `t` with `align` and no offset;
    /// 4. displays already on that side of `t` that `f` now overlaps move out beyond `f`, so
    ///    sticking F right-of A in A|B gives A|F|B, whichever display was the root.
    ///
    /// A mirror (`Side::Same`) switches `f` to `t`'s resolution when `f` has it.
    pub fn stick(
        &mut self,
        snap: &Snapshot,
        f: usize,
        t: usize,
        side: Side,
        align: Align,
    ) -> Result<CommitReport, EditError> {
        self.check_editable(f)?;
        if f == t {
            return Err(EditError::Refused(
                "A display cannot stick to itself.".to_owned(),
            ));
        }
        if !self.is_enabled(f) || !self.is_enabled(t) {
            return Err(EditError::Refused(
                "Both displays must be on to stick them.".to_owned(),
            ));
        }
        let before = self.clone();
        let mut notes = Vec::new();

        let own = self.subtree(f);
        if own.contains(&t) {
            let mut k = t;
            while let Some(p) = self.parent_of(k) {
                if p == f {
                    self.links[k] = None;
                    break;
                }
                k = p;
            }
        }
        self.reroot(t);

        if side == Side::Same {
            let target = self.outputs[t].mode.clone();
            let own_mode = self.outputs[f].mode.clone();
            if let (Some(target), Some(own_mode)) = (target, own_mode)
                && own_mode.size() != target.size()
            {
                let nearest = snap.outputs[f]
                    .rates(target.width, target.height)
                    .into_iter()
                    .min_by(|a, b| {
                        (a.refresh - target.refresh)
                            .abs()
                            .total_cmp(&(b.refresh - target.refresh).abs())
                    });
                match nearest {
                    Some(m) => self.outputs[f].mode = Some(m.clone()),
                    None => notes.push(format!(
                        "{} has no {}x{} mode; the smaller display shows the top-left part.",
                        self.names[f], target.width, target.height
                    )),
                }
            }
            self.links[f] = Some(Link {
                parent: t,
                side,
                align: Align::Start,
                offset: 0,
            });
        } else {
            self.links[f] = Some(Link {
                parent: t,
                side,
                align,
                offset: 0,
            });
            let mut moved = vec![f];
            let (mut cur, mut par) = (f, t);
            loop {
                self.resolve();
                let cur_rect = self.rect(cur);
                let next = (0..self.len()).find(|&x| {
                    x != cur
                        && !moved.contains(&x)
                        && self.is_enabled(x)
                        && self.links[x].is_some_and(|l| l.parent == par && l.side == side)
                        && self.rect(x).overlaps(&cur_rect)
                });
                let Some(x) = next else { break };
                let old = self.links[x].expect("checked above");
                self.links[x] = Some(Link { parent: cur, ..old });
                moved.push(x);
                par = cur;
                cur = x;
            }
        }
        Ok(self.commit(&before, EditKind::Relink).with_notes(notes))
    }

    /// Removes `f`'s link. It stays where it is as an anchor until it is stuck again.
    pub fn unstick(&mut self, f: usize) -> Result<CommitReport, EditError> {
        self.check_editable(f)?;
        if self.links[f].is_none() {
            return Err(EditError::NoChange(format!(
                "{} is not stuck to anything.",
                self.names[f]
            )));
        }
        let before = self.clone();
        self.links[f] = None;
        Ok(self.commit(&before, EditKind::Relink))
    }

    /// Turns display `d` off. Its heir is its mirror if it has one (which inherits `d`'s link
    /// and children unchanged), else its parent, else its largest child (which becomes an anchor
    /// in place). Other children stick to the heir the same way if that overlaps nothing, and
    /// become anchors in place otherwise. A primary `d` hands primary to the largest display.
    pub fn turn_off(&mut self, d: usize) -> Result<CommitReport, EditError> {
        self.check_editable(d)?;
        if !self.is_enabled(d) {
            return Err(EditError::NoChange(format!(
                "{} is already off.",
                self.names[d]
            )));
        }
        if self.enabled_count() == 1 {
            return Err(EditError::Refused(
                "The last enabled display stays on.".to_owned(),
            ));
        }
        let before = self.clone();
        let kids = self.children(d);
        let own_link = self.links[d];
        let mirror = kids
            .iter()
            .copied()
            .find(|&c| self.links[c].is_some_and(|l| l.side == Side::Same));
        let largest = kids
            .iter()
            .copied()
            .max_by_key(|&c| (self.rect(c).area(), Reverse(c)));
        let heir = mirror.or(own_link.map(|l| l.parent)).or(largest);

        self.outputs[d].enabled = false;
        self.links[d] = None;
        let mut records = Vec::new();
        for &x in &kids {
            let old = self.links[x].expect("child has a link");
            let new = if Some(x) == mirror {
                own_link
            } else if Some(x) == heir {
                None
            } else if mirror.is_some() {
                Some(Link {
                    parent: heir.expect("mirror is the heir"),
                    ..old
                })
            } else {
                let candidate = Link {
                    parent: heir.expect("a child implies an heir"),
                    ..old
                };
                self.links[x] = Some(candidate);
                self.fits_without_overlap(x).then_some(candidate)
            };
            self.links[x] = new;
            records.push((x, old, new));
        }

        let reference = heir
            .or_else(|| self.enabled().first().copied())
            .expect("another display is on");
        let offset = Point::new(
            before.outputs[d].pos.x - before.outputs[reference].pos.x,
            before.outputs[d].pos.y - before.outputs[reference].pos.y,
        );
        let was_primary = self.outputs[d].primary;
        let mut notes = Vec::new();
        if was_primary {
            self.outputs[d].primary = false;
            if let Some(p) = self
                .enabled()
                .into_iter()
                .max_by_key(|&i| (self.rect(i).area(), Reverse(i)))
            {
                self.outputs[p].primary = true;
                notes.push(format!("{} is primary now.", self.names[p]));
            }
        }
        self.restore[d] = Some(Restore {
            reference,
            offset,
            link: own_link,
            children: records,
            primary: was_primary,
        });
        Ok(self.commit(&before, EditKind::Relink).with_notes(notes))
    }

    /// Whether `x`'s subtree, placed by the current links, overlaps nothing outside it.
    fn fits_without_overlap(&self, x: usize) -> bool {
        let mut trial = self.clone();
        trial.resolve();
        let own = trial.subtree(x);
        let outside: Vec<usize> = trial
            .enabled()
            .into_iter()
            .filter(|i| !own.contains(i))
            .collect();
        !own.iter().any(|&a| {
            outside
                .iter()
                .any(|&b| !trial.are_mirrors(a, b) && trial.rect(a).overlaps(&trial.rect(b)))
        })
    }

    /// Turns display `d` back on. With a restore record whose reference display is on, `d`
    /// returns to its old place, link, children and primary flag. Otherwise it goes right of
    /// the rightmost display, top-aligned, in its preferred mode.
    pub fn turn_on(&mut self, snap: &Snapshot, d: usize) -> Result<CommitReport, EditError> {
        if self.is_enabled(d) {
            return Err(EditError::NoChange(format!(
                "{} is already on.",
                self.names[d]
            )));
        }
        if !snap.outputs[d].is_connected() {
            return Err(EditError::Refused(format!(
                "{} is not connected.",
                self.names[d]
            )));
        }
        let before = self.clone();
        let record = self.restore[d].take();
        match record {
            Some(r) if self.is_enabled(r.reference) && self.outputs[d].mode.is_some() => {
                let base = self.outputs[r.reference].pos;
                self.outputs[d].enabled = true;
                self.outputs[d].pos = Point::new(base.x + r.offset.x, base.y + r.offset.y);
                self.links[d] = r
                    .link
                    .filter(|l| self.is_enabled(l.parent) && l.parent != d);
                for (child, link, set) in r.children {
                    if self.is_enabled(child)
                        && self.links[child] == set
                        && !self.subtree(child).contains(&d)
                    {
                        self.links[child] = Some(link);
                    }
                }
                if r.primary {
                    for (k, o) in self.outputs.iter_mut().enumerate() {
                        o.primary = k == d;
                    }
                }
            }
            _ => {
                let mode = snap.outputs[d].preferred_mode().cloned().ok_or_else(|| {
                    EditError::Refused(format!("{} offers no modes.", self.names[d]))
                })?;
                let rightmost = self
                    .enabled()
                    .into_iter()
                    .filter(|&i| !self.is_mirror_child(i))
                    .max_by_key(|&i| (self.rect(i).right(), Reverse(self.rect(i).y), Reverse(i)))
                    .expect("another display is on");
                let r = self.rect(rightmost);
                let state = &mut self.outputs[d];
                state.enabled = true;
                state.mode = Some(mode);
                state.pos = Point::new(r.right(), r.y);
                self.links[d] = Some(Link {
                    parent: rightmost,
                    side: Side::RightOf,
                    align: Align::Start,
                    offset: 0,
                });
            }
        }
        Ok(self.commit(&before, EditKind::Relink))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_on_each_side() {
        let parent = Rect::new(1920, 0, 2560, 1440);
        let size = Size::new(1920, 1080);
        let link = |side, align, offset| Link {
            parent: 0,
            side,
            align,
            offset,
        };
        assert_eq!(
            place(size, parent, &link(Side::RightOf, Align::Start, 0)),
            Point::new(4480, 0)
        );
        assert_eq!(
            place(size, parent, &link(Side::LeftOf, Align::End, 0)),
            Point::new(0, 360)
        );
        assert_eq!(
            place(size, parent, &link(Side::LeftOf, Align::Center, 0)),
            Point::new(0, 180)
        );
        assert_eq!(
            place(size, parent, &link(Side::Below, Align::Center, 0)),
            Point::new(2240, 1440)
        );
        assert_eq!(
            place(size, parent, &link(Side::Above, Align::Start, 10)),
            Point::new(1930, -1080)
        );
        assert_eq!(
            place(size, parent, &link(Side::Same, Align::End, 99)),
            Point::new(1920, 0)
        );
    }

    #[test]
    fn placement_keeps_a_one_pixel_edge() {
        let parent = Rect::new(0, 0, 1920, 1080);
        let size = Size::new(1920, 1080);
        let far = Link {
            parent: 0,
            side: Side::RightOf,
            align: Align::Start,
            offset: 5000,
        };
        assert_eq!(place(size, parent, &far), Point::new(1920, 1079));
        let far = Link {
            parent: 0,
            side: Side::Below,
            align: Align::Start,
            offset: -5000,
        };
        assert_eq!(place(size, parent, &far), Point::new(-1919, 1080));
    }

    #[test]
    fn alignment_rule_and_ties() {
        let monitor = Rect::new(0, 0, 2560, 1440);
        // Centred below: Center wins over Start and End.
        assert_eq!(
            best_align(Side::Below, Rect::new(320, 1440, 1920, 1080), monitor),
            (Align::Center, 0)
        );
        // Nudged 10 px: still Center, now +10.
        assert_eq!(
            best_align(Side::Below, Rect::new(330, 1440, 1920, 1080), monitor),
            (Align::Center, 10)
        );
        // Same width below: all three tie at 0 and Center wins.
        assert_eq!(
            best_align(Side::Below, Rect::new(0, 1440, 2560, 1440), monitor),
            (Align::Center, 0)
        );
        // Same height beside: all three tie and Start wins, like xrandr's --right-of.
        assert_eq!(
            best_align(Side::RightOf, Rect::new(2560, 0, 2560, 1440), monitor),
            (Align::Start, 0)
        );
        // Bottom-aligned beside.
        assert_eq!(
            best_align(Side::LeftOf, Rect::new(-1920, 360, 1920, 1080), monitor),
            (Align::End, 0)
        );
    }
}
