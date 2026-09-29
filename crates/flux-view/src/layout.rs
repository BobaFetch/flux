//! The window layout: a tree of frames, ported from Vim's `window.c`.
//!
//! Every window has a statusline ('laststatus' 2), so a window's frame is one row taller than
//! its text. A window with another to its right has a separator column, so its frame is one
//! column wider. Sizes follow Vim's rules: `win_split`, `winframe_remove` (the space of a
//! closed window goes to the one below or to the right), `win_equal_rec` for 'equalalways', and
//! `frame_setheight` / `frame_new_height` for resizing.

/// Vim's 'winheight', 'winminheight', 'winwidth' and 'winminwidth' defaults.
const WINHEIGHT: usize = 1;
const WINMINHEIGHT: usize = 1;
const WINWIDTH: usize = 20;
const WINMINWIDTH: usize = 1;
const STATUS: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WindowId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    /// Heights only (Vim's `'v'`).
    Vertical,
    /// Widths only (`'h'`).
    Horizontal,
    /// Both (`'b'`).
    Both,
}

/// Where a window is on screen: its text area, and whether a separator column follows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub row: usize,
    pub col: usize,
    pub width: usize,
    /// Text rows, not counting the statusline below them.
    pub height: usize,
    pub vsep: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Leaf(WindowId),
    /// Side by side.
    Row(Vec<Frame>),
    /// Stacked.
    Col(Vec<Frame>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    kind: Kind,
    /// Including the separator column of a window with one.
    width: usize,
    /// Including statuslines.
    height: usize,
}

impl Frame {
    fn leaf(win: WindowId, width: usize, height: usize) -> Self {
        Self {
            kind: Kind::Leaf(win),
            width,
            height,
        }
    }

    fn children(&self) -> &[Frame] {
        match &self.kind {
            Kind::Leaf(_) => &[],
            Kind::Row(c) | Kind::Col(c) => c,
        }
    }

    fn children_mut(&mut self) -> &mut Vec<Frame> {
        match &mut self.kind {
            Kind::Leaf(_) => unreachable!("a leaf has no children"),
            Kind::Row(c) | Kind::Col(c) => c,
        }
    }

    fn is_leaf(&self) -> bool {
        matches!(self.kind, Kind::Leaf(_))
    }

    fn is_row(&self) -> bool {
        matches!(self.kind, Kind::Row(_))
    }

    fn has_win(&self, win: WindowId) -> bool {
        match &self.kind {
            Kind::Leaf(w) => *w == win,
            Kind::Row(c) | Kind::Col(c) => c.iter().any(|f| f.has_win(win)),
        }
    }

    fn windows(&self, out: &mut Vec<WindowId>) {
        match &self.kind {
            Kind::Leaf(w) => out.push(*w),
            Kind::Row(c) | Kind::Col(c) => c.iter().for_each(|f| f.windows(out)),
        }
    }

    fn first_window(&self) -> WindowId {
        match &self.kind {
            Kind::Leaf(w) => *w,
            Kind::Row(c) | Kind::Col(c) => c[0].first_window(),
        }
    }

    fn last_window(&self) -> WindowId {
        match &self.kind {
            Kind::Leaf(w) => *w,
            Kind::Row(c) | Kind::Col(c) => c[c.len() - 1].last_window(),
        }
    }

    /// Vim's `frame_minheight`: statuslines plus 'winminheight' (or 'winheight' for `cur`).
    fn min_height(&self, cur: Option<WindowId>) -> usize {
        match &self.kind {
            Kind::Leaf(w) => {
                STATUS
                    + if Some(*w) == cur {
                        WINHEIGHT
                    } else {
                        WINMINHEIGHT
                    }
            }
            Kind::Row(c) => c.iter().map(|f| f.min_height(cur)).max().unwrap_or(0),
            Kind::Col(c) => c.iter().map(|f| f.min_height(cur)).sum(),
        }
    }

    /// Vim's `frame_minwidth`. `right` says whether the frame touches the right edge of the
    /// screen, so its rightmost windows have no separator.
    fn min_width(&self, cur: Option<WindowId>, right: bool) -> usize {
        match &self.kind {
            Kind::Leaf(w) => {
                let vsep = usize::from(!right);
                vsep + if Some(*w) == cur {
                    WINWIDTH
                } else {
                    WINMINWIDTH
                }
            }
            Kind::Col(c) => c.iter().map(|f| f.min_width(cur, right)).max().unwrap_or(0),
            Kind::Row(c) => {
                let n = c.len();
                c.iter()
                    .enumerate()
                    .map(|(i, f)| f.min_width(cur, right && i == n - 1))
                    .sum()
            }
        }
    }

    /// Vim's `frame_new_height`: give the frame a new height, taking or giving rows from the
    /// bottom (or top, with `topfirst`) frame of a column first.
    fn new_height(&mut self, height: usize, topfirst: bool) {
        match &mut self.kind {
            Kind::Leaf(_) => {}
            Kind::Row(c) => c.iter_mut().for_each(|f| f.new_height(height, topfirst)),
            Kind::Col(c) => {
                let mut extra = height as isize - self.height as isize;
                let order: Vec<usize> = if topfirst {
                    (0..c.len()).collect()
                } else {
                    (0..c.len()).rev().collect()
                };
                if extra < 0 {
                    for i in order {
                        let min = c[i].min_height(None) as isize;
                        let h = c[i].height as isize;
                        if h + extra < min {
                            extra += h - min;
                            c[i].new_height(min as usize, topfirst);
                        } else {
                            c[i].new_height((h + extra) as usize, topfirst);
                            break;
                        }
                    }
                } else if extra > 0 {
                    let i = order[0];
                    let h = c[i].height + extra as usize;
                    c[i].new_height(h, topfirst);
                }
            }
        }
        self.height = height;
    }

    /// Vim's `frame_new_width`, taking or giving columns from the rightmost frame first.
    fn new_width(&mut self, width: usize, leftfirst: bool, right: bool) {
        match &mut self.kind {
            Kind::Leaf(_) => {}
            Kind::Col(c) => c
                .iter_mut()
                .for_each(|f| f.new_width(width, leftfirst, right)),
            Kind::Row(c) => {
                let n = c.len();
                let mut extra = width as isize - self.width as isize;
                let order: Vec<usize> = if leftfirst {
                    (0..n).collect()
                } else {
                    (0..n).rev().collect()
                };
                if extra < 0 {
                    for i in order {
                        let r = right && i == n - 1;
                        let min = c[i].min_width(None, r) as isize;
                        let w = c[i].width as isize;
                        if w + extra < min {
                            extra += w - min;
                            c[i].new_width(min as usize, leftfirst, r);
                        } else {
                            c[i].new_width((w + extra) as usize, leftfirst, r);
                            break;
                        }
                    }
                } else if extra > 0 {
                    let i = order[0];
                    let w = c[i].width + extra as usize;
                    c[i].new_width(w, leftfirst, right && i == n - 1);
                }
            }
        }
        self.width = width;
    }
}

/// A layout's shape: windows side by side (`Row`) or stacked (`Col`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutTree {
    Leaf(WindowId),
    Row(Vec<LayoutTree>),
    Col(Vec<LayoutTree>),
}

#[derive(Debug, Clone)]
pub struct Layout {
    root: Frame,
}

impl Layout {
    /// One window filling `width` x `height` (text rows plus its statusline).
    pub fn new(win: WindowId, width: usize, height: usize) -> Self {
        Self {
            root: Frame::leaf(win, width, height),
        }
    }

    /// Windows in Vim's order (`CTRL-W w`): top-left to bottom-right, depth first.
    pub fn windows(&self) -> Vec<WindowId> {
        let mut out = Vec::new();
        self.root.windows(&mut out);
        out
    }

    /// The shape of the layout, for tests and debugging.
    pub fn tree(&self) -> LayoutTree {
        fn go(f: &Frame) -> LayoutTree {
            match &f.kind {
                Kind::Leaf(w) => LayoutTree::Leaf(*w),
                Kind::Row(c) => LayoutTree::Row(c.iter().map(go).collect()),
                Kind::Col(c) => LayoutTree::Col(c.iter().map(go).collect()),
            }
        }
        go(&self.root)
    }

    pub fn contains(&self, win: WindowId) -> bool {
        self.root.has_win(win)
    }

    pub fn rects(&self) -> Vec<(WindowId, Rect)> {
        let mut out = Vec::new();
        fn walk(f: &Frame, row: usize, col: usize, right: bool, out: &mut Vec<(WindowId, Rect)>) {
            match &f.kind {
                Kind::Leaf(w) => {
                    let vsep = !right;
                    out.push((
                        *w,
                        Rect {
                            row,
                            col,
                            width: f.width - usize::from(vsep),
                            height: f.height.saturating_sub(STATUS),
                            vsep,
                        },
                    ));
                }
                Kind::Row(c) => {
                    let mut x = col;
                    for (i, child) in c.iter().enumerate() {
                        walk(child, row, x, right && i == c.len() - 1, out);
                        x += child.width;
                    }
                }
                Kind::Col(c) => {
                    let mut y = row;
                    for child in c {
                        walk(child, y, col, right, out);
                        y += child.height;
                    }
                }
            }
        }
        walk(&self.root, 0, 0, true, &mut out);
        out
    }

    pub fn rect(&self, win: WindowId) -> Option<Rect> {
        self.rects()
            .into_iter()
            .find(|(w, _)| *w == win)
            .map(|(_, r)| r)
    }

    /// Path of child indices from the root to `win`'s frame.
    fn path(&self, win: WindowId) -> Option<Vec<usize>> {
        fn find(f: &Frame, win: WindowId, path: &mut Vec<usize>) -> bool {
            match &f.kind {
                Kind::Leaf(w) => *w == win,
                Kind::Row(c) | Kind::Col(c) => {
                    for (i, child) in c.iter().enumerate() {
                        path.push(i);
                        if find(child, win, path) {
                            return true;
                        }
                        path.pop();
                    }
                    false
                }
            }
        }
        let mut path = Vec::new();
        find(&self.root, win, &mut path).then_some(path)
    }

    fn frame_at(&self, path: &[usize]) -> &Frame {
        path.iter().fold(&self.root, |f, &i| &f.children()[i])
    }

    fn frame_at_mut(&mut self, path: &[usize]) -> &mut Frame {
        path.iter()
            .fold(&mut self.root, |f, &i| &mut f.children_mut()[i])
    }

    /// Vim's `stl_connected`: whether `win`'s statusline runs into the statusline of the
    /// window to its right, so the cell below its separator belongs to the statusline.
    pub fn stl_connected(&self, win: WindowId) -> bool {
        let Some(path) = self.path(win) else {
            return false;
        };
        for depth in (0..path.len()).rev() {
            let parent = self.frame_at(&path[..depth]);
            let last = path[depth] + 1 == parent.children().len();
            if !last {
                return parent.is_row();
            }
        }
        false
    }

    /// Whether the frame at `path` touches the right edge of the screen.
    fn at_right(&self, path: &[usize]) -> bool {
        let mut f = &self.root;
        for &i in path {
            if f.is_row() && i != f.children().len() - 1 {
                return false;
            }
            f = &f.children()[i];
        }
        true
    }

    /// Split `cur`, putting `new` above it (or to its left when `vertical`), as `:split` and
    /// `:vsplit` do with 'nosplitbelow' and 'nosplitright'. `size` is a count given to the
    /// command; without one the windows are made equal ('equalalways'). Returns false when
    /// there's no room.
    pub fn split(
        &mut self,
        cur: WindowId,
        new: WindowId,
        vertical: bool,
        size: Option<usize>,
    ) -> bool {
        let Some(path) = self.path(cur) else {
            return false;
        };
        let right = self.at_right(&path);
        let old = self.frame_at(&path).clone();
        // Vim's `win_split_ins` with 'equalalways': there must be room for the new window on
        // the whole screen, counting the minimum size of every frame beside the ones `cur` is
        // in; windows are made equal afterwards when the split one gets too small, or when a
        // neighbour is bigger than either half.
        let parent_path = &path[..path.len().saturating_sub(1)];
        let siblings: Vec<Frame> = if path.is_empty() {
            Vec::new()
        } else {
            self.frame_at(parent_path).children().to_vec()
        };
        let idx = path.last().copied().unwrap_or(0);
        let mut do_equal;
        let (new_frame, old_frame) = if vertical {
            let old_width = (old.width - usize::from(!right)) as isize;
            let mut min = old.min_width(None, right) as isize;
            for depth in (0..path.len()).rev() {
                let parent = self.frame_at(&path[..depth]);
                if parent.is_row() {
                    for (i, f) in parent.children().iter().enumerate() {
                        if i != path[depth] {
                            let mut child = path[..depth].to_vec();
                            child.push(i);
                            min += f.min_width(None, self.at_right(&child)) as isize;
                        }
                    }
                }
            }
            let available = self.root.width as isize;
            let wmw1 = WINMINWIDTH as isize;
            if available < wmw1 + 1 + min {
                return false;
            }
            let mut new_size = size.map_or(old_width / 2, |s| s as isize);
            new_size = new_size.min(available - min - 1).max(wmw1);
            do_equal = old_width - new_size - 1 < WINMINWIDTH as isize;
            if !do_equal && size.is_none() {
                do_equal = siblings.iter().enumerate().any(|(i, f)| {
                    let mut child = parent_path.to_vec();
                    child.push(i);
                    let text = f.width as isize - isize::from(!self.at_right(&child));
                    i != idx && f.is_leaf() && (text > new_size || text > old_width - new_size - 1)
                });
            }
            let new_size = new_size as usize;
            // The new window, on the left, gets a separator.
            let new_frame = Frame::leaf(new, new_size + 1, old.height);
            let mut old_frame = old.clone();
            old_frame.new_width(old.width.saturating_sub(new_size + 1), false, right);
            (new_frame, old_frame)
        } else {
            let old_height = (old.height - STATUS) as isize;
            let mut min = old.min_height(None) as isize;
            for depth in (0..path.len()).rev() {
                let parent = self.frame_at(&path[..depth]);
                if !parent.is_row() {
                    for (i, f) in parent.children().iter().enumerate() {
                        if i != path[depth] {
                            min += f.min_height(None) as isize;
                        }
                    }
                }
            }
            let available = self.root.height as isize;
            let wmh1 = WINMINHEIGHT as isize;
            if available < wmh1 + STATUS as isize + min {
                return false;
            }
            let mut new_size = size.map_or(old_height / 2, |s| s as isize);
            new_size = new_size.min(available - min - STATUS as isize).max(wmh1);
            do_equal = old_height - new_size - (STATUS as isize) < WINMINHEIGHT as isize;
            if !do_equal && size.is_none() {
                do_equal = siblings.iter().enumerate().any(|(i, f)| {
                    let text = (f.height - STATUS) as isize;
                    i != idx
                        && f.is_leaf()
                        && (text > new_size || text > old_height - new_size - STATUS as isize)
                });
            }
            let new_size = new_size as usize;
            let new_frame = Frame::leaf(new, old.width, new_size + STATUS);
            let mut old_frame = old.clone();
            old_frame.new_height(
                old.height.saturating_sub(new_size + STATUS).max(STATUS),
                false,
            );
            (new_frame, old_frame)
        };

        // Insert next to `cur` in a parent of the same kind, or make one.
        let same_kind_parent = !path.is_empty() && self.frame_at(parent_path).is_row() == vertical;
        if same_kind_parent {
            let idx = path[path.len() - 1];
            let parent = self.frame_at_mut(parent_path);
            parent.children_mut()[idx] = old_frame;
            parent.children_mut().insert(idx, new_frame);
        } else {
            let slot = self.frame_at_mut(&path);
            let (width, height) = (old.width, old.height);
            *slot = Frame {
                kind: if vertical {
                    Kind::Row(vec![new_frame, old_frame])
                } else {
                    Kind::Col(vec![new_frame, old_frame])
                },
                width,
                height,
            };
        }
        if do_equal {
            self.equalize(
                new,
                if vertical {
                    Dir::Horizontal
                } else {
                    Dir::Vertical
                },
                true,
            );
        }
        true
    }

    /// Remove `win`. Its space goes to the next window in its row or column (the previous one
    /// if it was last), which is returned as the one to make current. `None` when it's the
    /// only window.
    pub fn close(&mut self, win: WindowId) -> Option<WindowId> {
        let path = self.path(win)?;
        if path.is_empty() {
            return None;
        }
        let (idx, parent_path) = (path[path.len() - 1], path[..path.len() - 1].to_vec());
        let parent = self.frame_at_mut(&parent_path);
        let is_row = parent.is_row();
        let children = parent.children_mut();
        let closed = children.remove(idx);
        let (target, next) = if idx < children.len() {
            (idx, true)
        } else {
            (idx - 1, false)
        };
        let parent_right = self.at_right(&parent_path);
        let parent = self.frame_at_mut(&parent_path);
        let n = parent.children().len();
        let child = &mut parent.children_mut()[target];
        if is_row {
            let w = child.width + closed.width;
            child.new_width(w, next, parent_right && target == n - 1);
        } else {
            let h = child.height + closed.height;
            child.new_height(h, next);
        }
        let gets_space = if next {
            child.first_window()
        } else {
            child.last_window()
        };
        // A row or column left with one frame is replaced by it.
        if n == 1 {
            let only = parent.children_mut().remove(0);
            let (w, h) = (parent.width, parent.height);
            *parent = Frame {
                width: w,
                height: h,
                ..only
            };
        }
        self.flatten();
        let dir = if is_row {
            Dir::Horizontal
        } else {
            Dir::Vertical
        };
        self.equalize(gets_space, dir, true);
        Some(gets_space)
    }

    /// Merge a row inside a row (or a column inside a column) into its parent.
    fn flatten(&mut self) {
        fn go(f: &mut Frame) {
            let is_row = f.is_row();
            if let Kind::Row(c) | Kind::Col(c) = &mut f.kind {
                c.iter_mut().for_each(go);
                let mut out = Vec::new();
                for child in c.drain(..) {
                    match child.kind {
                        Kind::Row(inner) if is_row => out.extend(inner),
                        Kind::Col(inner) if !is_row => out.extend(inner),
                        kind => out.push(Frame { kind, ..child }),
                    }
                }
                *c = out;
            }
        }
        go(&mut self.root);
    }

    /// Only `win` remains.
    pub fn only(&mut self, win: WindowId) {
        let (w, h) = (self.root.width, self.root.height);
        self.root = Frame::leaf(win, w, h);
    }

    /// Vim's `win_equal`: make windows the same size, favoring `cur` ('winheight',
    /// 'winwidth'). `current` limits it to the frames around `cur` as Vim does after a split or
    /// close.
    pub fn equalize(&mut self, cur: WindowId, dir: Dir, current: bool) {
        let (w, h) = (self.root.width, self.root.height);
        equal_rec(&mut self.root, cur, current, dir, 0, 0, w, h, w, true);
    }

    /// Resize the whole layout for a new screen size, like Vim's `shell_new_rows` and
    /// `shell_new_columns`: the bottom and rightmost windows take the difference.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.root.new_height(height, false);
        self.root.new_width(width, false, true);
    }

    /// Vim's `win_setheight`: make `win` `height` text rows, taking rows from the windows
    /// below it and then above it.
    pub fn set_height(&mut self, win: WindowId, height: usize) {
        let Some(path) = self.path(win) else { return };
        self.set_frame_height(&path, height.max(WINMINHEIGHT) + STATUS);
    }

    fn set_frame_height(&mut self, path: &[usize], height: usize) {
        if path.is_empty() {
            return;
        }
        let parent_path = &path[..path.len() - 1];
        let idx = path[path.len() - 1];
        if self.frame_at(parent_path).is_row() {
            // A row has one height: resize the row within its own column.
            self.set_frame_height(parent_path, height);
            return;
        }
        let parent = self.frame_at_mut(parent_path);
        let others_min: usize = parent
            .children()
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != idx)
            .map(|(_, f)| f.min_height(None))
            .sum();
        let height = height.min(parent.height - others_min);
        let children = parent.children_mut();
        let mut take = height as isize - children[idx].height as isize;
        children[idx].new_height(height, false);
        let after: Vec<usize> = (idx + 1..children.len()).collect();
        let before: Vec<usize> = (0..idx).rev().collect();
        for i in after.into_iter().chain(before) {
            if take == 0 {
                break;
            }
            let min = children[i].min_height(None) as isize;
            let h = children[i].height as isize;
            if h - take < min {
                take -= h - min;
                children[i].new_height(min as usize, false);
            } else {
                children[i].new_height((h - take) as usize, false);
                take = 0;
            }
        }
    }

    /// Vim's `win_setwidth`: make `win` `width` columns wide.
    pub fn set_width(&mut self, win: WindowId, width: usize) {
        let Some(path) = self.path(win) else { return };
        let vsep = usize::from(!self.at_right(&path));
        self.set_frame_width(&path, width.max(WINMINWIDTH) + vsep);
    }

    fn set_frame_width(&mut self, path: &[usize], width: usize) {
        if path.is_empty() {
            return;
        }
        let parent_path = &path[..path.len() - 1];
        let idx = path[path.len() - 1];
        if !self.frame_at(parent_path).is_row() {
            self.set_frame_width(parent_path, width);
            return;
        }
        let parent_right = self.at_right(parent_path);
        let parent = self.frame_at_mut(parent_path);
        let n = parent.children().len();
        let right = |i: usize| parent_right && i == n - 1;
        let others_min: usize = parent
            .children()
            .iter()
            .enumerate()
            .filter(|&(i, _)| i != idx)
            .map(|(i, f)| f.min_width(None, right(i)))
            .sum();
        let width = width.min(parent.width - others_min);
        let children = parent.children_mut();
        let mut take = width as isize - children[idx].width as isize;
        children[idx].new_width(width, false, right(idx));
        let after: Vec<usize> = (idx + 1..n).collect();
        let before: Vec<usize> = (0..idx).rev().collect();
        for i in after.into_iter().chain(before) {
            if take == 0 {
                break;
            }
            let min = children[i].min_width(None, right(i)) as isize;
            let w = children[i].width as isize;
            if w - take < min {
                take -= w - min;
                children[i].new_width(min as usize, false, right(i));
            } else {
                children[i].new_width((w - take) as usize, false, right(i));
                take = 0;
            }
        }
    }

    /// `CTRL-W x`: swap `win` with the next window in its row or column (the previous one if
    /// it's last), windows keeping their sizes. Returns the window now in `win`'s place.
    pub fn exchange(&mut self, win: WindowId) -> Option<WindowId> {
        let path = self.path(win)?;
        let (&idx, parent_path) = path.split_last()?;
        let parent = self.frame_at(parent_path);
        let other = if idx + 1 < parent.children().len() {
            idx + 1
        } else {
            idx.checked_sub(1)?
        };
        let Kind::Leaf(other_win) = parent.children()[other].kind else {
            return None;
        };
        // The windows swap places and keep their sizes; the separator stays where it was.
        self.reorder_keeping_text_size(parent_path, |c| c.swap(idx, other));
        Some(other_win)
    }

    /// Reorder the windows of the row or column at `parent_path` the way Vim's `win_exchange`
    /// and `win_rotate` do: each window keeps its text size, while separators stay with the
    /// positions (the last window of a row at the right edge has none).
    fn reorder_keeping_text_size(
        &mut self,
        parent_path: &[usize],
        reorder: impl FnOnce(&mut Vec<Frame>),
    ) {
        let n = self.frame_at(parent_path).children().len();
        let vseps: Vec<usize> = (0..n)
            .map(|i| {
                let mut p = parent_path.to_vec();
                p.push(i);
                usize::from(!self.at_right(&p))
            })
            .collect();
        let c = self.frame_at_mut(parent_path).children_mut();
        for (f, vsep) in c.iter_mut().zip(&vseps) {
            f.width -= vsep;
        }
        reorder(c);
        for (f, vsep) in c.iter_mut().zip(&vseps) {
            f.width += vsep;
        }
    }

    /// `CTRL-W r` (`down`) and `CTRL-W R`: rotate the windows in `win`'s row or column. The
    /// windows keep their sizes.
    pub fn rotate(&mut self, win: WindowId, down: bool) -> bool {
        let Some(path) = self.path(win) else {
            return false;
        };
        let Some((_, parent_path)) = path.split_last() else {
            return false;
        };
        let parent = self.frame_at_mut(parent_path);
        if parent
            .children()
            .iter()
            .any(|f| !matches!(f.kind, Kind::Leaf(_)))
        {
            return false;
        }
        self.reorder_keeping_text_size(parent_path, |c| {
            if down {
                c.rotate_right(1);
            } else {
                c.rotate_left(1);
            }
        });
        true
    }

    /// `CTRL-W H`/`J`/`K`/`L`: move `win` to the far left/bottom/top/right, full height or
    /// width, then make windows equal.
    pub fn move_to_edge(&mut self, win: WindowId, edge: char) {
        if self.windows().len() < 2 {
            return;
        }
        self.close_quietly(win);
        let (w, h) = (self.root.width, self.root.height);
        let vertical = matches!(edge, 'H' | 'L');
        let new_frame = Frame::leaf(win, w, h);
        let old = std::mem::replace(&mut self.root, Frame::leaf(win, w, h));
        let (first, second) = if matches!(edge, 'H' | 'K') {
            (new_frame, old)
        } else {
            (old, new_frame)
        };
        self.root = Frame {
            kind: if vertical {
                Kind::Row(vec![first, second])
            } else {
                Kind::Col(vec![first, second])
            },
            width: w,
            height: h,
        };
        self.flatten();
        // Give the moved window its half, then equalize everything.
        self.equalize(win, Dir::Both, false);
    }

    /// Remove `win` without equalizing (for moving it).
    fn close_quietly(&mut self, win: WindowId) {
        let Some(path) = self.path(win) else { return };
        let Some((&idx, parent_path)) = path.split_last() else {
            return;
        };
        let parent_path = parent_path.to_vec();
        let parent = self.frame_at_mut(&parent_path);
        parent.children_mut().remove(idx);
        if parent.children().len() == 1 {
            let only = parent.children_mut().remove(0);
            let (w, h) = (parent.width, parent.height);
            *parent = Frame {
                width: w,
                height: h,
                ..only
            };
        }
        self.flatten();
    }

    /// The window `count` steps away in `dir` (`h`/`j`/`k`/`l`) from `win`, aiming at screen
    /// position `at` (the cursor) when choosing between windows, like Vim's `win_vert_neighbor`
    /// and `win_horz_neighbor`.
    pub fn neighbor(
        &self,
        win: WindowId,
        dir: char,
        count: usize,
        at: (usize, usize),
    ) -> Option<WindowId> {
        let rects = self.rects();
        let mut current = win;
        let (mut row, mut col) = at;
        for _ in 0..count.max(1) {
            let r = rects.iter().find(|(w, _)| *w == current)?.1;
            let (tr, tc) = match dir {
                'j' => (r.row + r.height + STATUS, col),
                'k' => (r.row.checked_sub(STATUS + 1)?, col),
                'l' => (row, r.col + r.width + usize::from(r.vsep)),
                'h' => (row, r.col.checked_sub(2)?),
                _ => return None,
            };
            let next = rects.iter().find(|(_, q)| {
                tr >= q.row
                    && tr < q.row + q.height + STATUS
                    && tc >= q.col
                    && tc <= q.col + q.width
            });
            match next {
                Some((w, q)) => {
                    current = *w;
                    row = row.clamp(q.row, q.row + q.height.saturating_sub(1));
                    col = col.clamp(q.col, q.col + q.width.saturating_sub(1));
                }
                None => break,
            }
        }
        (current != win).then_some(current)
    }
}

/// Vim's `win_equal_rec`.
#[allow(clippy::too_many_arguments)]
fn equal_rec(
    f: &mut Frame,
    cur: WindowId,
    current: bool,
    dir: Dir,
    col: usize,
    row: usize,
    width: usize,
    height: usize,
    columns: usize,
    is_root: bool,
) {
    let right = col + width == columns;
    match &mut f.kind {
        Kind::Leaf(_) => {
            f.width = width;
            f.height = height;
        }
        Kind::Row(_) => {
            f.width = width;
            f.height = height;
            let mut room = 0isize;
            let mut totwincount = 0isize;
            let mut next_size = 0isize;
            let mut has_cur = false;
            let extra_sep = usize::from(right) as isize;
            if dir != Dir::Vertical {
                let n = f.min_width(None, right) as isize;
                totwincount = (n + extra_sep) / (WINMINWIDTH as isize + 1);
                has_cur = f.has_win(cur);
                let m = f.min_width(Some(cur), right) as isize;
                room = width as isize - m;
                if room < 0 {
                    next_size = WINWIDTH as isize + room;
                    room = 0;
                } else if !has_cur {
                    next_size = 0;
                } else if totwincount > 1
                    && (room + (totwincount - 2)) / (totwincount - 1) > WINWIDTH as isize
                {
                    next_size = (room
                        + WINWIDTH as isize
                        + (totwincount - 1) * WINMINWIDTH as isize
                        + (totwincount - 1))
                        / totwincount;
                    room -= next_size - WINWIDTH as isize;
                } else {
                    next_size = WINWIDTH as isize;
                }
                if has_cur {
                    totwincount -= 1;
                }
            }
            let children = f.children_mut();
            let count = children.len();
            let (mut x, mut left) = (col, width as isize);
            for (i, child) in children.iter_mut().enumerate() {
                let last = i == count - 1;
                let child_right = right && last;
                let mut wincount = 1;
                let new_size = if last {
                    left
                } else if dir == Dir::Vertical {
                    child.width as isize
                } else {
                    let n = child.min_width(None, child_right) as isize;
                    wincount = (n + if last { extra_sep } else { 0 }) / (WINMINWIDTH as isize + 1);
                    let m = child.min_width(Some(cur), child_right) as isize;
                    let hnc = has_cur && child.has_win(cur);
                    if hnc {
                        wincount -= 1;
                    }
                    let mut size = if totwincount == 0 {
                        room
                    } else {
                        (wincount * room + totwincount / 2) / totwincount
                    };
                    if hnc {
                        next_size -= WINWIDTH as isize - (m - n);
                        next_size = next_size.max(0);
                        size += next_size;
                        room -= size - next_size;
                    } else {
                        room -= size;
                    }
                    size + n
                };
                let new_size = new_size.max(0) as usize;
                let skip = current
                    && dir == Dir::Horizontal
                    && !is_root
                    && new_size == child.width
                    && !child.has_win(cur);
                if !skip {
                    equal_rec(
                        child, cur, current, dir, x, row, new_size, height, columns, false,
                    );
                }
                x += new_size;
                left -= new_size as isize;
                totwincount -= wincount;
            }
        }
        Kind::Col(_) => {
            f.width = width;
            f.height = height;
            let mut room = 0isize;
            let mut totwincount = 0isize;
            let mut next_size = 0isize;
            let mut has_cur = false;
            if dir != Dir::Horizontal {
                let n = f.min_height(None) as isize;
                totwincount = n / (WINMINHEIGHT as isize + 1);
                has_cur = f.has_win(cur);
                let m = f.min_height(Some(cur)) as isize;
                room = height as isize - m;
                if room < 0 {
                    next_size = WINHEIGHT as isize + room;
                    room = 0;
                } else if !has_cur {
                    next_size = 0;
                } else if totwincount > 1
                    && (room + (totwincount - 2)) / (totwincount - 1) > WINHEIGHT as isize
                {
                    next_size = (room
                        + WINHEIGHT as isize
                        + (totwincount - 1) * WINMINHEIGHT as isize
                        + (totwincount - 1))
                        / totwincount;
                    room -= next_size - WINHEIGHT as isize;
                } else {
                    next_size = WINHEIGHT as isize;
                }
                if has_cur {
                    totwincount -= 1;
                }
            }
            let children = f.children_mut();
            let count = children.len();
            let (mut y, mut left) = (row, height as isize);
            for (i, child) in children.iter_mut().enumerate() {
                let last = i == count - 1;
                let mut wincount = 1;
                let new_size = if last {
                    left
                } else if dir == Dir::Horizontal {
                    child.height as isize
                } else {
                    let n = child.min_height(None) as isize;
                    wincount = n / (WINMINHEIGHT as isize + 1);
                    let m = child.min_height(Some(cur)) as isize;
                    let hnc = has_cur && child.has_win(cur);
                    if hnc {
                        wincount -= 1;
                    }
                    let mut size = if totwincount == 0 {
                        room
                    } else {
                        (wincount * room + totwincount / 2) / totwincount
                    };
                    if hnc {
                        next_size -= WINHEIGHT as isize - (m - n);
                        size += next_size;
                        room -= size - next_size;
                    } else {
                        room -= size;
                    }
                    size + n
                };
                let new_size = new_size.max(0) as usize;
                let skip = current
                    && dir == Dir::Vertical
                    && !is_root
                    && new_size == child.height
                    && !child.has_win(cur);
                if !skip {
                    equal_rec(
                        child, cur, current, dir, col, y, width, new_size, columns, false,
                    );
                }
                y += new_size;
                left -= new_size as isize;
                totwincount -= wincount;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: fn(usize) -> WindowId = WindowId;

    /// Text sizes `(height, width)` of every window in order, as the oracle reports them.
    fn sizes(l: &Layout) -> Vec<(usize, usize)> {
        l.rects()
            .into_iter()
            .map(|(_, r)| (r.height, r.width))
            .collect()
    }

    /// An 80x24 screen: 23 rows for windows (the last row is the command line).
    fn screen() -> Layout {
        Layout::new(W(1), 80, 23)
    }

    #[test]
    fn splits_match_neovim() {
        let mut l = screen();
        l.split(W(1), W(2), false, None);
        assert_eq!(sizes(&l), [(11, 80), (10, 80)]);
        l.split(W(2), W(3), false, None);
        assert_eq!(sizes(&l), [(7, 80), (7, 80), (6, 80)]);
        l.split(W(3), W(4), false, None);
        assert_eq!(sizes(&l), [(5, 80), (5, 80), (5, 80), (4, 80)]);

        let mut l = screen();
        l.split(W(1), W(2), true, None);
        assert_eq!(sizes(&l), [(22, 40), (22, 39)]);
        l.split(W(2), W(3), true, None);
        assert_eq!(sizes(&l), [(22, 26), (22, 26), (22, 26)]);
    }

    #[test]
    fn many_splits() {
        let mut l = screen();
        let mut cur = W(1);
        for i in 2..=7 {
            l.split(cur, W(i), false, None);
            cur = W(i);
        }
        assert_eq!(
            sizes(&l).iter().map(|s| s.0).collect::<Vec<_>>(),
            [3, 2, 2, 2, 2, 3, 2]
        );
    }

    #[test]
    fn mixed_splits_and_counts() {
        let mut l = screen();
        l.split(W(1), W(2), false, None);
        l.split(W(2), W(3), true, None);
        assert_eq!(sizes(&l), [(11, 40), (11, 39), (10, 80)]);

        let mut l = screen();
        l.split(W(1), W(2), false, Some(3));
        assert_eq!(sizes(&l), [(3, 80), (18, 80)]);
        let mut l = screen();
        l.split(W(1), W(2), true, Some(20));
        assert_eq!(sizes(&l), [(22, 20), (22, 59)]);
    }

    #[test]
    fn close_gives_space_below_or_right() {
        let mut l = screen();
        l.split(W(1), W(2), false, None);
        l.split(W(2), W(3), false, None);
        // [3, 2, 1] top to bottom; close the middle one.
        assert_eq!(l.windows(), [W(3), W(2), W(1)]);
        assert_eq!(l.close(W(2)), Some(W(1)));
        assert_eq!(sizes(&l), [(10, 80), (11, 80)]);

        let mut l = screen();
        l.split(W(1), W(2), true, None);
        l.split(W(2), W(3), true, None);
        assert_eq!(l.close(W(2)), Some(W(1)));
        assert_eq!(sizes(&l), [(22, 39), (22, 40)]);
    }

    #[test]
    fn resizing() {
        let mut l = screen();
        l.split(W(1), W(2), false, None);
        l.set_height(W(2), 16);
        assert_eq!(sizes(&l), [(16, 80), (5, 80)]);
        l.set_height(W(2), 100);
        assert_eq!(sizes(&l), [(20, 80), (1, 80)]);
        l.equalize(W(2), Dir::Both, false);
        assert_eq!(sizes(&l), [(11, 80), (10, 80)]);

        let mut l = screen();
        l.split(W(1), W(2), true, None);
        l.set_width(W(2), 100);
        assert_eq!(sizes(&l), [(22, 78), (22, 1)]);
        l.set_width(W(2), 35);
        assert_eq!(sizes(&l), [(22, 35), (22, 44)]);
    }

    #[test]
    fn neighbors_follow_the_cursor() {
        let mut l = screen();
        l.split(W(1), W(2), true, None); // 2 left, 1 right
        l.split(W(2), W(3), false, None); // 3 top-left, 2 bottom-left
        assert_eq!(l.neighbor(W(3), 'l', 1, (0, 0)), Some(W(1)));
        assert_eq!(l.neighbor(W(1), 'h', 1, (0, 41)), Some(W(3)));
        assert_eq!(l.neighbor(W(1), 'h', 1, (15, 41)), Some(W(2)));
        assert_eq!(l.neighbor(W(3), 'j', 1, (0, 0)), Some(W(2)));
        assert_eq!(l.neighbor(W(3), 'k', 1, (0, 0)), None);
    }

    #[test]
    fn move_to_edge() {
        let mut l = screen();
        l.split(W(1), W(2), false, None);
        l.move_to_edge(W(2), 'L');
        assert_eq!(l.windows(), [W(1), W(2)]);
        assert_eq!(sizes(&l), [(22, 39), (22, 40)]);
    }

    #[test]
    fn screen_resize_takes_from_the_bottom() {
        let mut l = screen();
        l.split(W(1), W(2), false, None);
        // 13 rows: the bottom window keeps its minimum (1 text row and a statusline).
        l.resize(80, 13);
        assert_eq!(sizes(&l), [(10, 80), (1, 80)]);
    }
}
