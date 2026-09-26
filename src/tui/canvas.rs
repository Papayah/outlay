//! The layout drawn to scale: one box per display, written straight into the buffer so labels
//! stay readable, with a viewport that holds still while you edit.
//!
//! Every box edge is rounded on its own (`round(px * scale)`), so displays that touch share one
//! border line, and the borders merge into `┬ ┴ ┼` joins.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::symbols::merge::MergeStrategy;
use ratatui::widgets::{Block, BorderType, Clear, Widget};

use crate::model::Snapshot;
use crate::model::geometry::{self, Dir, Point, bbox};
use crate::model::layout::Layout;
use crate::model::links::Side;

use super::theme::Theme;

/// Maps X pixels to terminal cells. It keeps its scale and origin between frames and re-fits
/// only when the layout leaves the view, fills less than half of it, or the area changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Viewport {
    /// Columns per X pixel. Rows per pixel are `scale / aspect`.
    scale: f64,
    /// The X coordinates shown at the area's top-left cell.
    origin: (f64, f64),
    /// Cell height divided by cell width.
    aspect: f64,
    area: Rect,
    fitted: bool,
}

/// A box in cells, both ends inclusive. It may reach outside the area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cells {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Cells {
    pub fn width(&self) -> i32 {
        self.x1 - self.x0 + 1
    }

    pub fn height(&self) -> i32 {
        self.y1 - self.y0 + 1
    }

    /// The part inside `area`, as a ratatui rectangle.
    fn clip(&self, area: Rect) -> Option<Rect> {
        let x0 = self.x0.max(i32::from(area.x));
        let y0 = self.y0.max(i32::from(area.y));
        let x1 = self.x1.min(i32::from(area.right()) - 1);
        let y1 = self.y1.min(i32::from(area.bottom()) - 1);
        (x0 <= x1 && y0 <= y1).then(|| {
            Rect::new(
                x0 as u16,
                y0 as u16,
                (x1 - x0 + 1) as u16,
                (y1 - y0 + 1) as u16,
            )
        })
    }
}

impl Viewport {
    /// Re-fits when needed so `bounds` is fully visible in `area`.
    pub fn update(&mut self, area: Rect, bounds: Option<geometry::Rect>, aspect: f64) {
        let Some(b) = bounds else { return };
        let stale = !self.fitted || self.area != area || (self.aspect - aspect).abs() > 1e-9;
        if stale || !self.shows(b) || self.fill(b) < 0.5 {
            self.fit(area, b, aspect);
        }
    }

    /// Forces a re-fit on the next update (`z`).
    pub fn refit(&mut self) {
        self.fitted = false;
    }

    /// Follows a normalisation shift, so displays that did not move stay still on screen.
    pub fn shift(&mut self, by: Point) {
        self.origin.0 += f64::from(by.x);
        self.origin.1 += f64::from(by.y);
    }

    /// Scales `b` to fill `area`, keeping a free column on each side, and centres it.
    pub fn fit(&mut self, area: Rect, b: geometry::Rect, aspect: f64) {
        let aspect = if aspect > 0.0 { aspect } else { 2.0 };
        let span_x = f64::from(area.width.saturating_sub(3).max(1));
        let span_y = f64::from(area.height.saturating_sub(1).max(1));
        let (bw, bh) = (f64::from(b.w.max(1)), f64::from(b.h.max(1)));
        let scale = (span_x / bw).min(span_y * aspect / bh);
        let free_x = f64::from(area.width.saturating_sub(1)) / scale - bw;
        let free_y = f64::from(area.height.saturating_sub(1)) * aspect / scale - bh;
        *self = Viewport {
            scale,
            origin: (f64::from(b.x) - free_x / 2.0, f64::from(b.y) - free_y / 2.0),
            aspect,
            area,
            fitted: true,
        };
    }

    pub fn col(&self, px: i32) -> i32 {
        i32::from(self.area.x) + ((f64::from(px) - self.origin.0) * self.scale).round() as i32
    }

    pub fn row(&self, py: i32) -> i32 {
        i32::from(self.area.y)
            + ((f64::from(py) - self.origin.1) * self.scale / self.aspect).round() as i32
    }

    pub fn cells(&self, r: geometry::Rect) -> Cells {
        Cells {
            x0: self.col(r.x),
            y0: self.row(r.y),
            x1: self.col(r.right()),
            y1: self.row(r.bottom()),
        }
    }

    fn shows(&self, b: geometry::Rect) -> bool {
        let c = self.cells(b);
        let a = self.area;
        c.x0 >= i32::from(a.x)
            && c.y0 >= i32::from(a.y)
            && c.x1 < i32::from(a.right())
            && c.y1 < i32::from(a.bottom())
    }

    /// How much of the area `b` fills, along its fuller axis.
    fn fill(&self, b: geometry::Rect) -> f64 {
        let c = self.cells(b);
        let w = f64::from(c.width()) / f64::from(self.area.width.max(1));
        let h = f64::from(c.height()) / f64::from(self.area.height.max(1));
        w.max(h)
    }
}

/// Everything the canvas draws.
pub struct Scene<'a> {
    pub layout: &'a Layout,
    pub snap: &'a Snapshot,
    pub theme: Theme,
    /// The focused display: thick border and a reversed title chip, drawn last; its moving set
    /// takes its colour.
    pub focus: Option<usize>,
    /// The stick target: thick border and a bold, underlined title, drawn just before the focused
    /// display. With `double_borders`, a double border instead.
    pub target: Option<usize>,
    /// The old look: the target gets a double border.
    pub double_borders: bool,
    /// Where displays would go, as dashed outlines.
    pub ghosts: &'a [(usize, geometry::Rect)],
    /// Outputs with pending changes, indexed like the layout.
    pub pending: &'a [bool],
}

impl Scene<'_> {
    /// The rectangle the viewport must show: the enabled displays and the ghosts.
    pub fn bounds(&self) -> Option<geometry::Rect> {
        let on = self
            .layout
            .enabled()
            .into_iter()
            .map(|i| self.layout.rect(i));
        bbox(on.chain(self.ghosts.iter().map(|&(_, r)| r)))
    }

    /// One box per mirror group: the displays that are not mirroring another one. A display
    /// with a ghost is drawn only as its ghost, unless it is focused: its current place is
    /// what the preview is compared with.
    fn boxes(&self) -> Vec<usize> {
        self.layout
            .enabled()
            .into_iter()
            .filter(|&i| !self.layout.is_mirror_child(i))
            .filter(|&i| Some(i) == self.focus || !self.ghosts.iter().any(|&(g, _)| g == i))
            .collect()
    }

    fn number(&self, i: usize) -> String {
        self.layout.numbers[i].map_or_else(|| "?".to_owned(), |n| n.to_string())
    }

    /// `2 DP-1-2 ★ ●`, or `2=4 DP-1-2=DP-1-3` for a mirror group.
    fn title(&self, group: &[usize]) -> String {
        let numbers: Vec<String> = group.iter().map(|&i| self.number(i)).collect();
        let names: Vec<&str> = group
            .iter()
            .map(|&i| self.layout.names[i].as_str())
            .collect();
        let mut title = format!("{} {}", numbers.join("="), names.join("="));
        if group.iter().any(|&i| self.layout.outputs[i].primary) {
            title.push_str(" ★");
        }
        if group
            .iter()
            .any(|&i| self.pending.get(i).copied().unwrap_or(false))
        {
            title.push_str(" ●");
        }
        title
    }

    /// `1920x1080@165.01`, with a rotation marker and a scale badge: `1080x1920@60.00 ↺ ×1.5`.
    fn mode_line(&self, i: usize) -> String {
        let st = &self.layout.outputs[i];
        let Some(mode) = &st.mode else {
            return String::new();
        };
        let mut line = if self.snap.outputs[i].mode(mode.xid).is_some() {
            mode.summary()
        } else {
            format!("{}x{}", mode.width, mode.height)
        };
        let marker = match st.rotation {
            crate::model::Rotation::Normal => "",
            crate::model::Rotation::Left => " ↺",
            crate::model::Rotation::Right => " ↻",
            crate::model::Rotation::Inverted => " ⇅",
        };
        line.push_str(marker);
        match st.transform.scale_factors() {
            Some((sx, sy)) if (sx - sy).abs() < 1e-6 && (sx - 1.0).abs() > 1e-6 => {
                line.push_str(&format!(" ×{}", trim_float(sx)));
            }
            Some((sx, sy)) if (sx - sy).abs() >= 1e-6 => {
                line.push_str(&format!(" ×{}x{}", trim_float(sx), trim_float(sy)));
            }
            Some(_) => {}
            None => line.push_str(" ×?"),
        }
        line
    }

    fn colour(&self, i: usize) -> Style {
        self.theme.output(self.layout.numbers[i])
    }
}

/// `1.5`, `2`, `1.25`
fn trim_float(v: f64) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Cuts `text` to `width` characters, ending in `…` when it had to cut.
pub fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = text.chars().take(width - 1).collect();
    out.push('…');
    out
}

/// Writes `text` centred on row `y` of `area`.
fn centred(buf: &mut Buffer, area: Rect, y: u16, text: &str, style: Style) {
    let text = truncate(text, usize::from(area.width));
    let len = text.chars().count() as u16;
    let x = area.x + (area.width - len) / 2;
    buf.set_string(x, y, text, style);
}

/// The focused display's title: reversed, with a space on each side. When that does not fit, the
/// reversed title fills the width, cut with `…`.
fn chip(buf: &mut Buffer, area: Rect, y: u16, text: &str, style: Style) {
    let width = usize::from(area.width);
    let padded = format!(" {text} ");
    let text = if padded.chars().count() <= width {
        padded
    } else {
        format!("{:^width$}", truncate(text, width))
    };
    centred(buf, area, y, &text, style.add_modifier(Modifier::REVERSED));
}

/// The glyph at a stick seam, pointing from the child to its parent.
fn link_glyph(side: Side) -> &'static str {
    match side {
        Side::RightOf => "◂",
        Side::LeftOf => "▸",
        Side::Below => "▴",
        Side::Above => "▾",
        Side::Same => "=",
    }
}

/// The cells of the edge `a` and `b` share, from one end to the other.
fn seam_cells(view: &Viewport, a: geometry::Rect, b: geometry::Rect) -> Option<Vec<(i32, i32)>> {
    let (dir, _) = a.shared_edge(&b)?;
    let cells = match dir {
        Dir::Left | Dir::Right => {
            let x = view.col(if dir == Dir::Right { a.right() } else { a.x });
            let (y0, y1) = (view.row(a.y.max(b.y)), view.row(a.bottom().min(b.bottom())));
            (y0..=y1).map(|y| (x, y)).collect()
        }
        Dir::Up | Dir::Down => {
            let y = view.row(if dir == Dir::Down { a.bottom() } else { a.y });
            let (x0, x1) = (view.col(a.x.max(b.x)), view.col(a.right().min(b.right())));
            (x0..=x1).map(|x| (x, y)).collect()
        }
    };
    Some(cells)
}

/// Clears what a ghost covers, except the box lines under its border, which it merges with. Their
/// style is reset, so no title's reverse video or underline leaks into the ghost.
fn clear_under_ghost(buf: &mut Buffer, rect: Rect) {
    if rect.width < 3 || rect.height < 3 {
        Clear.render(rect, buf);
        return;
    }
    Clear.render(
        Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2),
        buf,
    );
    for y in rect.top()..rect.bottom() {
        for x in rect.left()..rect.right() {
            let ring = y == rect.top()
                || y == rect.bottom() - 1
                || x == rect.left()
                || x == rect.right() - 1;
            if !ring {
                continue;
            }
            let cell = &mut buf[(x, y)];
            let line = cell
                .symbol()
                .chars()
                .all(|c| ('\u{2500}'..='\u{257f}').contains(&c));
            if line {
                cell.set_style(Style::reset());
            } else {
                cell.reset();
            }
        }
    }
}

fn cell_in(area: Rect, x: i32, y: i32) -> Option<(u16, u16)> {
    let inside = x >= i32::from(area.x)
        && y >= i32::from(area.y)
        && x < i32::from(area.right())
        && y < i32::from(area.bottom());
    inside.then_some((x as u16, y as u16))
}

/// Draws the scene into `area`. The viewport must already be updated for this area.
pub fn render(scene: &Scene, view: &Viewport, area: Rect, buf: &mut Buffer) {
    let layout = scene.layout;
    let boxes = scene.boxes();
    let focus_root = scene
        .focus
        .filter(|&f| layout.is_enabled(f))
        .map(|f| layout.mirror_root(f));
    let moving = focus_root.map(|f| layout.moving_set(f)).unwrap_or_default();
    let target_root = scene
        .target
        .filter(|&t| layout.is_enabled(t))
        .map(|t| layout.mirror_root(t))
        .filter(|&t| Some(t) != focus_root && boxes.contains(&t));

    let tint = |root: usize| match focus_root {
        Some(f) if moving.contains(&root) => scene.colour(f),
        _ => scene.colour(root),
    };
    // The box in cells when it is big enough for a border; a smaller one shows only its numbers,
    // drawn after the seams so no glyph covers them.
    let framed = |root: usize| -> Option<Rect> {
        let cells = view.cells(layout.rect(root));
        if cells.width() < 3 || cells.height() < 3 {
            return None;
        }
        cells.clip(area)
    };
    let draw_numbers = |buf: &mut Buffer, root: usize| {
        let cells = view.cells(layout.rect(root));
        if cells.width() >= 3 && cells.height() >= 3 {
            return;
        }
        let (x, y) = ((cells.x0 + cells.x1) / 2, (cells.y0 + cells.y1) / 2);
        if let Some((x, y)) = cell_in(area, x, y) {
            let group = layout.mirror_group(root);
            let numbers: Vec<String> = group.iter().map(|&i| scene.number(i)).collect();
            let mut style = tint(root).add_modifier(Modifier::BOLD);
            if Some(root) == focus_root {
                style = style.add_modifier(Modifier::REVERSED);
            }
            buf.set_string(x, y, numbers.join("="), style);
        }
    };
    let draw_border = |buf: &mut Buffer, root: usize| {
        let Some(rect) = framed(root) else {
            return;
        };
        let border = if Some(root) == focus_root {
            BorderType::Thick
        } else if Some(root) == target_root {
            if scene.double_borders {
                BorderType::Double
            } else {
                BorderType::Thick
            }
        } else {
            BorderType::Plain
        };
        Block::bordered()
            .border_type(border)
            .border_style(tint(root))
            .merge_borders(MergeStrategy::Fuzzy)
            .render(rect, buf);
    };
    let draw_labels = |buf: &mut Buffer, root: usize| {
        let cells = view.cells(layout.rect(root));
        if cells.width() < 3 || cells.height() < 3 {
            return;
        }
        let Some(rect) = cells.clip(area) else { return };
        let inner = Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2);
        if inner.width == 0 || inner.height == 0 {
            return;
        }
        let focused = Some(root) == focus_root;
        let title_style = if focused {
            tint(root).add_modifier(Modifier::BOLD)
        } else if Some(root) == target_root && !scene.double_borders {
            tint(root).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            tint(root)
        };
        let model = scene.snap.outputs[root].label();
        let group = layout.mirror_group(root);
        let mut lines: Vec<(String, Style)> = vec![(scene.title(&group), title_style)];
        match inner.height {
            1 => {}
            2 => lines.push((scene.mode_line(root), Style::new())),
            _ => {
                if !model.is_empty() {
                    lines.push((model, Style::new()));
                }
                lines.push((scene.mode_line(root), Style::new()));
            }
        }
        lines.truncate(usize::from(inner.height));
        let top = inner.y + (inner.height - lines.len() as u16) / 2;
        for (k, (text, style)) in lines.iter().enumerate() {
            if k == 0 && focused {
                chip(buf, inner, top, text, *style);
            } else {
                centred(buf, inner, top + k as u16, text, *style);
            }
        }
    };

    // Borders first: the others, then the stick target, then the focused one, so the thick lines
    // win the merges; then the overlap hatch; then the labels on top of both.
    let order: Vec<usize> = boxes
        .iter()
        .copied()
        .filter(|&b| Some(b) != focus_root && Some(b) != target_root)
        .chain(target_root)
        .chain(focus_root)
        .collect();
    for &root in &order {
        draw_border(buf, root);
    }

    // Overlaps: hatch the free cells both displays cover and colour everything there red.
    for (a, b) in layout.overlapping_pairs() {
        let (ra, rb) = (layout.rect(a), layout.rect(b));
        let x = ra.x.max(rb.x);
        let y = ra.y.max(rb.y);
        let w = ra.right().min(rb.right()) - x;
        let h = ra.bottom().min(rb.bottom()) - y;
        let c = view.cells(geometry::Rect::new(x, y, w, h));
        for cy in c.y0..=c.y1 {
            for cx in c.x0..=c.x1 {
                let Some(pos) = cell_in(area, cx, cy) else {
                    continue;
                };
                let cell = &mut buf[pos];
                if cell.symbol() == " " {
                    cell.set_symbol("░");
                }
                cell.set_style(scene.theme.overlap());
            }
        }
    }

    for &root in &order {
        draw_labels(buf, root);
    }

    // Seams: the shared edges, where the mouse crosses.
    for (k, &a) in boxes.iter().enumerate() {
        for &b in &boxes[k + 1..] {
            let Some(cells) = seam_cells(view, layout.rect(a), layout.rect(b)) else {
                continue;
            };
            for (x, y) in cells {
                if let Some(pos) = cell_in(area, x, y) {
                    buf[pos].set_style(scene.theme.seam());
                }
            }
        }
    }

    // Stick links: a glyph at the middle of each seam, pointing from child to parent.
    for &child in &boxes {
        let Some(link) = layout.links[child] else {
            continue;
        };
        if link.side == Side::Same || !layout.is_enabled(link.parent) {
            continue;
        }
        let parent = layout.mirror_root(link.parent);
        let Some(cells) = seam_cells(view, layout.rect(parent), layout.rect(child)) else {
            continue;
        };
        let (x, y) = cells[cells.len() / 2];
        if let Some(pos) = cell_in(area, x, y) {
            buf[pos]
                .set_symbol(link_glyph(link.side))
                .set_style(scene.theme.seam().add_modifier(Modifier::BOLD));
        }
    }

    for &root in &order {
        draw_numbers(buf, root);
    }

    // Ghosts: where the displays would go. Each covers what lies under it, so it reads as the
    // new position. Their borders merge with the lines under them, so a ghost meets a box in a
    // clean join; everything else under a ghost is cleared first.
    let ghosts: Vec<(usize, Rect)> = scene
        .ghosts
        .iter()
        .filter_map(|&(i, r)| view.cells(r).clip(area).map(|rect| (i, rect)))
        .collect();
    for &(_, rect) in &ghosts {
        clear_under_ghost(buf, rect);
    }
    for &(i, rect) in &ghosts {
        let style = scene.colour(i).add_modifier(Modifier::BOLD);
        if rect.width < 3 || rect.height < 3 {
            buf.set_string(rect.x, rect.y, scene.number(i), style);
            continue;
        }
        Block::bordered()
            .border_type(BorderType::LightDoubleDashed)
            .border_style(style)
            .merge_borders(MergeStrategy::Fuzzy)
            .render(rect, buf);
    }
    for &(i, rect) in &ghosts {
        let style = scene.colour(i).add_modifier(Modifier::BOLD);
        let inner = Rect::new(
            rect.x + 1,
            rect.y + 1,
            rect.width.saturating_sub(2),
            rect.height.saturating_sub(2),
        );
        if inner.height > 0 && inner.width > 0 {
            let label = format!("{} {}", scene.number(i), layout.names[i]);
            centred(buf, inner, inner.y + (inner.height - 1) / 2, &label, style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touching_displays_share_a_border_column() {
        let mut view = Viewport::default();
        let area = Rect::new(0, 0, 60, 20);
        let a = geometry::Rect::new(0, 0, 1920, 1080);
        let b = geometry::Rect::new(1920, 0, 2560, 1440);
        view.update(area, bbox([a, b]), 2.0);
        assert_eq!(view.cells(a).x1, view.cells(b).x0);
        let all = view.cells(bbox([a, b]).unwrap());
        assert!(all.x0 >= 1 && all.x1 <= 58, "{all:?}");
        assert!(all.y0 >= 0 && all.y1 <= 19, "{all:?}");
    }

    #[test]
    fn the_view_holds_still_until_the_layout_leaves_it() {
        let mut view = Viewport::default();
        let area = Rect::new(0, 0, 80, 24);
        let a = geometry::Rect::new(0, 0, 1920, 1080);
        let b = geometry::Rect::new(1920, 0, 1920, 1080);
        view.update(area, bbox([a, b]), 2.0);
        let before = view.clone();

        // A small nudge stays inside: nothing moves on screen.
        let nudged = b.translated(10, 0);
        view.update(area, bbox([a, nudged]), 2.0);
        assert_eq!(view, before);

        // Normalisation shifted everything by 100 px; the origin follows it.
        view.shift(Point::new(100, 0));
        view.update(
            area,
            bbox([a.translated(100, 0), b.translated(100, 0)]),
            2.0,
        );
        assert_eq!(view.cells(a.translated(100, 0)), before.cells(a));

        // Growing past the edge re-fits.
        let wide = geometry::Rect::new(0, 0, 9000, 1080);
        view.update(area, Some(wide), 2.0);
        assert_ne!(view, before);
        let c = view.cells(wide);
        assert!(c.x1 < 80);

        // Shrinking below half the view re-fits too.
        let fitted = view.clone();
        view.update(area, Some(geometry::Rect::new(0, 0, 1000, 500)), 2.0);
        assert_ne!(view, fitted);
    }

    #[test]
    fn truncation_marks_the_cut() {
        assert_eq!(truncate("DP-1-2", 10), "DP-1-2");
        assert_eq!(truncate("HDMI-1-0 Philips", 8), "HDMI-1-…");
        assert_eq!(truncate("abc", 0), "");
        assert_eq!(trim_float(1.5), "1.5");
        assert_eq!(trim_float(2.0), "2");
    }
}
