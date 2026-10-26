use crate::colorpick::Picker;
use crate::i18n::t;
use crate::render::{self, Camera, Frame, ModelOpts};
use crate::settings::{parse_hex, Settings};
use crate::{skin, tools};
use gtk::{cairo, gdk, gio, glib, prelude::*};
use image::{Rgba, RgbaImage};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

#[derive(Clone, Copy, PartialEq)]
pub enum Tool {
    Pencil,
    Eraser,
    Fill,
    Picker,
    Replace,
    Select,
}

// Minecraft dye colours.
const PALETTE: [u32; 16] = [
    0xF9FFFE, 0x9D9D97, 0x474F52, 0x1D1D21, 0x835432, 0xB02E26, 0xF9801D, 0xFED83D, 0x80C71F, 0x5E7C16, 0x169C9C, 0x3AB3DA, 0x3C44AA,
    0x8932B8, 0xC74EBD, 0xF38BAA,
];
const RECENT: usize = 8;

/// One open skin (a tab).
pub struct Doc {
    pub id: u64,
    pub skin: RgbaImage,
    pub slim: bool,
    pub file: Option<PathBuf>,
    pub dirty: bool,
    undo: Vec<RgbaImage>,
    redo: Vec<RgbaImage>,
    before: RgbaImage,
    history: Vec<RgbaImage>,
}

impl Doc {
    fn title(&self) -> String {
        title_of(&self.file, self.dirty)
    }
}

fn title_of(file: &Option<PathBuf>, dirty: bool) -> String {
    let name = file.as_ref().and_then(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned());
    format!("{}{}", name.unwrap_or_else(|| t("untitled").into()), if dirty { " •" } else { "" })
}

enum SelDrag {
    New((i32, i32)),
    Move { start: (i32, i32), lifted: RgbaImage, orig: [u32; 4], off: (i32, i32) },
}

pub struct State {
    pub settings: Settings,
    // The active document lives in these fields; the other tabs wait in `tabs`.
    pub id: u64,
    pub skin: RgbaImage,
    pub slim: bool,
    pub file: Option<PathBuf>,
    pub dirty: bool,
    undo: Vec<RgbaImage>,
    redo: Vec<RgbaImage>,
    /// The skin as it was when opened or last saved ("before / after").
    before: RgbaImage,
    /// Every stroke's starting state, for the timelapse.
    history: Vec<RgbaImage>,
    tabs: Vec<Option<Doc>>,
    active: usize,
    next_id: u64,
    stroke_open: bool,
    pub tool: Tool,
    pub color: [u8; 4],
    pub brush: u32,
    pub round: bool,
    pressure: f64,
    stylus_eraser: bool,
    recent: Vec<[u8; 4]>,
    pub sel: Option<[u32; 4]>,
    sel_drag: Option<SelDrag>,
    clip: Option<RgbaImage>,
    zoom: f64,
    pan: (f64, f64),
    pub show_base: bool,
    pub show_overlay: bool,
    pub voxels: bool,
    pub paint3d: bool,
    pub paint_overlay: bool,
    pub walk: bool,
    pub spin: bool,
    /// 0 studio, 1 like in game, 2 day, 3 night with a torch.
    pub scene: u32,
    pub item: usize,
    pub armor: usize,
    pub explode: f32,
    pub compare: Option<usize>,
    pub before_after: Option<f32>,
    /// Running emote (kind, start time on `clock`): 0 wave, 1 jump, 2 cheer.
    emote: Option<(u8, f32)>,
    clock: f32,
    yaw: f32,
    pitch: f32,
    dist: f32,
    drag_rot: Option<(f32, f32)>,
    last_texel: Option<(i32, i32)>,
    anim_t: f32,
    last_tick: f64,
    frame: Option<Frame>,
    frame_q: f64,
    /// Called after every change (redraws, window title). Must not borrow the state.
    pub listeners: Vec<Box<dyn Fn(&State)>>,
}

pub type St = Rc<RefCell<State>>;

impl State {
    pub fn new(settings: Settings) -> Self {
        let s = &settings;
        Self {
            id: 1,
            skin: skin::starter(),
            slim: s.slim_default,
            file: None,
            dirty: false,
            undo: vec![],
            redo: vec![],
            before: skin::starter(),
            history: vec![],
            tabs: vec![None],
            active: 0,
            next_id: 2,
            stroke_open: false,
            tool: Tool::Pencil,
            color: [176, 46, 38, 255],
            brush: 1,
            round: false,
            pressure: 1.,
            stylus_eraser: false,
            recent: vec![],
            sel: None,
            sel_drag: None,
            clip: None,
            zoom: 1.,
            pan: (0., 0.),
            show_base: true,
            show_overlay: true,
            voxels: s.layers3d,
            paint3d: false,
            paint_overlay: false,
            walk: s.walk,
            spin: s.auto_rotate,
            scene: 0,
            item: 0,
            armor: 0,
            explode: 0.,
            compare: None,
            before_after: None,
            emote: None,
            clock: 0.,
            yaw: 0.5,
            pitch: 0.25,
            dist: 70.,
            drag_rot: None,
            last_texel: None,
            anim_t: 0.,
            last_tick: 0.,
            frame: None,
            frame_q: 1.,
            listeners: vec![],
            settings,
        }
    }

    pub fn refresh(&self) {
        for l in &self.listeners {
            l(self);
        }
    }

    pub fn title(&self) -> String {
        title_of(&self.file, self.dirty)
    }

    // --- tabs ---

    fn take_current(&mut self) -> Doc {
        Doc {
            id: self.id,
            skin: std::mem::replace(&mut self.skin, RgbaImage::new(64, 64)),
            slim: self.slim,
            file: self.file.take(),
            dirty: self.dirty,
            undo: std::mem::take(&mut self.undo),
            redo: std::mem::take(&mut self.redo),
            before: std::mem::replace(&mut self.before, RgbaImage::new(64, 64)),
            history: std::mem::take(&mut self.history),
        }
    }

    fn load_doc(&mut self, d: Doc) {
        (self.id, self.skin, self.slim, self.file, self.dirty, self.undo, self.redo, self.before, self.history) = (d.id, d.skin, d.slim, d.file, d.dirty, d.undo, d.redo, d.before, d.history);
        self.compare = None;
        self.sel = None;
        self.sel_drag = None;
    }

    fn pristine(&self) -> bool {
        self.file.is_none() && !self.dirty && self.undo.is_empty()
    }

    /// Opens a skin in a new tab (or reuses the current tab if it is an untouched new skin).
    pub fn open_doc(&mut self, img: RgbaImage, slim: bool, file: Option<PathBuf>, force_new_tab: bool) {
        let id = self.next_id;
        self.next_id += 1;
        let doc = Doc { id, before: img.clone(), skin: img, slim, file, dirty: false, undo: vec![], redo: vec![], history: vec![] };
        if self.pristine() && !force_new_tab {
            self.load_doc(doc);
        } else {
            let cur = self.take_current();
            self.tabs[self.active] = Some(cur);
            self.tabs.push(None);
            self.active = self.tabs.len() - 1;
            self.load_doc(doc);
        }
        self.refresh();
    }

    pub fn switch_tab(&mut self, i: usize) {
        if i == self.active || i >= self.tabs.len() {
            return;
        }
        let cur = self.take_current();
        self.tabs[self.active] = Some(cur);
        let next = self.tabs[i].take().unwrap();
        self.active = i;
        self.load_doc(next);
        self.refresh();
    }

    pub fn tab_dirty(&self, i: usize) -> bool {
        if i == self.active { self.dirty } else { self.tabs.get(i).and_then(|d| d.as_ref()).is_some_and(|d| d.dirty) }
    }

    pub fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        if self.tabs.len() == 1 {
            let slim = self.settings.slim_default;
            let id = self.next_id;
            self.next_id += 1;
            let st = skin::starter();
            self.load_doc(Doc { id, skin: st.clone(), before: st, slim, file: None, dirty: false, undo: vec![], redo: vec![], history: vec![] });
        } else if i == self.active {
            self.tabs.remove(i);
            self.active = i.min(self.tabs.len() - 1);
            let next = self.tabs[self.active].take().unwrap();
            self.load_doc(next);
        } else {
            self.tabs.remove(i);
            if i < self.active {
                self.active -= 1;
            }
        }
        self.refresh();
    }

    pub fn tabs(&self) -> Vec<String> {
        (0..self.tabs.len()).map(|i| if i == self.active { self.title() } else { self.tabs[i].as_ref().map(|d| d.title()).unwrap_or_default() }).collect()
    }

    pub fn active_tab(&self) -> usize {
        self.active
    }

    pub fn any_dirty(&self) -> bool {
        (0..self.tabs.len()).any(|i| self.tab_dirty(i))
    }

    /// (id, skin, slim, file) of every unsaved tab, for crash recovery.
    pub fn dirty_docs(&self) -> Vec<(u64, &RgbaImage, bool, Option<&PathBuf>)> {
        let mut v = vec![];
        if self.dirty {
            v.push((self.id, &self.skin, self.slim, self.file.as_ref()));
        }
        for d in self.tabs.iter().flatten().filter(|d| d.dirty) {
            v.push((d.id, &d.skin, d.slim, d.file.as_ref()));
        }
        v
    }

    /// Swaps the skin of an untouched new tab (used when Steve finishes downloading).
    pub fn replace_pristine(&mut self, img: RgbaImage) {
        if self.pristine() {
            self.before = img.clone();
            self.skin = img;
            self.refresh();
        }
    }

    pub fn mark_saved(&mut self, path: PathBuf) {
        self.file = Some(path);
        self.dirty = false;
        self.before = self.skin.clone();
        self.refresh();
    }

    /// Skin and model of another tab (for side-by-side comparison).
    pub fn tab_skin(&self, i: usize) -> Option<(RgbaImage, bool)> {
        if i == self.active { None } else { self.tabs.get(i)?.as_ref().map(|d| (d.skin.clone(), d.slim)) }
    }

    pub fn set_view(&mut self, yaw: f32, pitch: f32) {
        (self.yaw, self.pitch) = (yaw, pitch);
        self.refresh();
    }

    /// Renders the 3D scene as configured. `img` overrides the edited skin (for "before").
    pub fn render_view(&self, w: usize, h: usize, yaw: f32, anim_t: f32, img: Option<&RgbaImage>, transparent: bool) -> Frame {
        let skin_img = img.unwrap_or(&self.skin);
        let opts = ModelOpts { slim: self.slim, base: self.show_base, overlay: self.show_overlay, voxels: self.voxels, thickness: self.settings.thickness as f32 };
        let mut pose = if self.walk { render::walk_pose(anim_t, 0.7) } else { [[0.; 3]; 6] };
        let jump = self.emote_pose(&mut pose);
        let other = self.compare.and_then(|i| self.tab_skin(i));
        let x = if other.is_some() { -11. } else { 0. };
        let x3 = [x, jump, 0.];
        // Owned meshes/textures first, then borrowed into Models.
        let mut meshes: Vec<(Vec<render::Quad>, RgbaImage, bool, [f32; 3], bool)> = vec![(render::build(skin_img, &opts), skin_img.clone(), self.slim, x3, true)];
        if self.armor > 0 {
            let tex = tools::armor_texture(tools::ARMORS[self.armor].1, self.slim);
            meshes.push((render::shell(self.slim, |i| if i >= skin::RLEG { 0.6 } else { 1.05 }), tex, self.slim, x3, false));
        }
        if self.item > 0 {
            let spr = tools::item_sprite(self.item, skin_img, self.slim);
            // Held in the right hand, pointing forward and up (0.6 units per sprite pixel).
            let k = 0.6;
            let q = render::extrude(&spr, [-6.3, 10.5 + 13. * k, 1. - 2. * k], [0., 0., k], [0., -k, 0.], [k, 0., 0.], skin::RARM as u8);
            meshes.push((q, spr, self.slim, x3, false));
        }
        if let Some((oimg, oslim)) = other {
            let o2 = ModelOpts { slim: oslim, ..opts };
            meshes.push((render::build(&oimg, &o2), oimg, oslim, [11., 0., 0.], false));
        }
        let models: Vec<render::Model> = meshes.iter().map(|(q, i, slim, off, pick)| render::Model { quads: q, img: i, slim: *slim, offset: *off, pose, explode: self.explode, pick: *pick }).collect();
        let cam = Camera { yaw, pitch: self.pitch, dist: self.dist, fov: self.settings.fov as f32, target: [0., 16., 0.] };
        let (top, bottom, light) = match self.scene {
            0 if transparent => ([0; 4], [0; 4], if self.settings.shading { render::Light::Soft } else { render::Light::Flat }),
            1 | 2 => ([111, 168, 255, 255], [207, 227, 255, 255], render::Light::Minecraft),
            3 => ([5, 8, 24, 255], [27, 36, 72, 255], render::Light::Night),
            _ => {
                let bg = parse_hex(&self.settings.bg);
                (bg, bg, if self.settings.shading { render::Light::Soft } else { render::Light::Flat })
            }
        };
        render::render_scene(&models, &cam, w, h, top, bottom, light)
    }

    /// Every recorded stroke plus the current skin.
    pub fn timelapse(&self) -> Vec<RgbaImage> {
        let mut v = self.history.clone();
        v.push(self.skin.clone());
        v
    }

    pub fn emote(&mut self, kind: u8) {
        self.emote = Some((kind, self.clock));
        self.refresh();
    }

    /// Extra pose + vertical offset of a running emote.
    fn emote_pose(&self, pose: &mut render::Pose) -> f32 {
        let Some((kind, t0)) = self.emote else { return 0. };
        let e = self.clock - t0;
        if e > 1.8 {
            return 0.;
        }
        let fade = (1. - (e / 1.8).powi(4)).max(0.);
        let pi = std::f32::consts::PI;
        match kind {
            0 => {
                pose[skin::RARM] = [0., 0., (-2.7 + (e * 9.).sin() * 0.35) * fade];
                0.
            }
            1 => {
                pose[skin::RARM][2] = -0.5 * fade;
                pose[skin::LARM][2] = 0.5 * fade;
                ((e * pi / 0.9).sin().abs()) * 5. * fade
            }
            _ => {
                pose[skin::RARM] = [0., 0., -2.8 * fade];
                pose[skin::LARM] = [0., 0., 2.8 * fade];
                ((e * pi / 0.6).sin().abs()) * 2.5 * fade
            }
        }
    }

    pub fn anim_state(&self) -> (f32, f32) {
        (self.yaw, self.anim_t)
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.refresh();
    }

    // --- painting ---

    fn begin(&mut self) {
        self.stroke_open = true;
        self.last_texel = None;
        if self.tool == Tool::Pencil && !self.recent.contains(&self.color) {
            self.recent.insert(0, self.color);
            self.recent.truncate(RECENT);
        }
    }

    /// Undo snapshot is taken lazily, so clicks that change nothing don't fill history.
    fn snapshot(&mut self) {
        if self.stroke_open {
            self.stroke_open = false;
            self.undo.push(self.skin.clone());
            self.history.push(self.skin.clone());
            if self.history.len() > 600 {
                // Keep the whole story but at half the frame rate.
                self.history = std::mem::take(&mut self.history).into_iter().step_by(2).collect();
            }
            let lim = self.settings.undo_limit.max(1) as usize;
            if self.undo.len() > lim {
                self.undo.drain(..self.undo.len() - lim);
            }
            self.redo.clear();
            self.dirty = true;
        }
    }

    /// Snapshot for one-shot operations (dialogs, selection commands).
    pub fn checkpoint(&mut self) {
        self.stroke_open = true;
        self.snapshot();
    }

    fn tool_now(&self) -> Tool {
        if self.stylus_eraser && matches!(self.tool, Tool::Pencil | Tool::Eraser) { Tool::Eraser } else { self.tool }
    }

    fn brush_now(&self) -> u32 {
        if self.settings.tablet && self.pressure < 1. { 1 + ((self.brush.max(1) - 1) as f64 * self.pressure).round() as u32 } else { self.brush.max(1) }
    }

    /// Applies the current tool at texel (x, y). Returns a colour if the eyedropper picked one.
    fn apply(&mut self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= 64 || y >= 64 {
            return None;
        }
        let cur = self.skin.get_pixel(x, y).0;
        match self.tool_now() {
            Tool::Picker => {
                self.color = cur;
                return Some(cur);
            }
            Tool::Pencil | Tool::Eraser => {
                let c = if self.tool_now() == Tool::Pencil { self.color } else { [0; 4] };
                // The brush never spills over into a neighbouring UV face.
                let [rx, ry, rw, rh] = skin::face_at(self.slim, x, y).map(|f| f.2).unwrap_or([0, 0, 64, 64]);
                for (px, py) in tools::brush(x as i32, y as i32, self.brush_now(), self.round) {
                    if px < rx as i32 || py < ry as i32 || px >= (rx + rw) as i32 || py >= (ry + rh) as i32 {
                        continue;
                    }
                    if self.skin.get_pixel(px as u32, py as u32).0 != c {
                        self.snapshot();
                        self.skin.put_pixel(px as u32, py as u32, Rgba(c));
                    }
                }
            }
            Tool::Fill => {
                let rect = skin::face_at(self.slim, x, y).map(|f| f.2).unwrap_or([0, 0, 64, 64]);
                if cur != self.color {
                    self.snapshot();
                    flood(&mut self.skin, x, y, rect, self.color);
                }
            }
            Tool::Replace => {
                if cur != self.color {
                    self.snapshot();
                    tools::replace_color(&mut self.skin, cur, self.color);
                }
            }
            Tool::Select => {}
        }
        self.refresh();
        None
    }

    pub fn undo(&mut self) {
        if let Some(img) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.skin, img));
            self.dirty = true;
            self.refresh();
        }
    }

    pub fn redo(&mut self) {
        if let Some(img) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.skin, img));
            self.dirty = true;
            self.refresh();
        }
    }

    fn paint_line(&mut self, x: i32, y: i32) {
        let (x0, y0) = self.last_texel.unwrap_or((x, y));
        for (px, py) in line(x0, y0, x, y) {
            if px >= 0 && py >= 0 {
                self.apply(px as u32, py as u32);
            }
        }
        self.last_texel = Some((x, y));
    }

    fn pick_3d(&self, x: f64, y: f64) -> Option<(u32, u32, bool)> {
        let q = self.frame_q;
        self.frame.as_ref()?.pick_at((x * q) as usize, (y * q) as usize)
    }

    fn paint_3d(&mut self, x: f64, y: f64) -> Option<[u8; 4]> {
        let (u, v, ov) = self.pick_3d(x, y)?;
        let (u, v) = if self.paint_overlay && !ov && self.tool != Tool::Picker { skin::to_overlay(self.slim, u, v) } else { (u, v) };
        self.apply(u, v)
    }

    // --- selection ---

    pub fn sel_copy(&mut self) {
        if let Some(r) = self.sel {
            self.clip = Some(tools::crop(&self.skin, r));
        }
    }

    pub fn has_clip(&self) -> bool {
        self.clip.is_some()
    }

    pub fn sel_paste(&mut self) {
        let Some(c) = self.clip.clone() else { return };
        let (x, y) = self.sel.map(|r| (r[0], r[1])).unwrap_or((0, 0));
        self.checkpoint();
        tools::stamp(&mut self.skin, &c, x as i32, y as i32);
        self.sel = Some([x, y, c.width().min(64 - x), c.height().min(64 - y)]);
        self.refresh();
    }

    pub fn sel_delete(&mut self) {
        if let Some(r) = self.sel {
            self.checkpoint();
            tools::clear_rect(&mut self.skin, r);
            self.refresh();
        }
    }

    pub fn sel_transform(&mut self, f: fn(&RgbaImage) -> RgbaImage) {
        let Some(r) = self.sel else { return };
        let piece = f(&tools::crop(&self.skin, r));
        self.checkpoint();
        tools::clear_rect(&mut self.skin, r);
        tools::stamp(&mut self.skin, &piece, r[0] as i32, r[1] as i32);
        self.sel = Some([r[0], r[1], piece.width().min(64 - r[0]), piece.height().min(64 - r[1])]);
        self.refresh();
    }

    fn sel_begin(&mut self, t: (i32, i32)) {
        let inside = self.sel.is_some_and(|[x, y, w, h]| t.0 >= x as i32 && t.1 >= y as i32 && t.0 < (x + w) as i32 && t.1 < (y + h) as i32);
        if inside {
            let r = self.sel.unwrap();
            let lifted = tools::crop(&self.skin, r);
            self.checkpoint();
            tools::clear_rect(&mut self.skin, r);
            self.sel_drag = Some(SelDrag::Move { start: t, lifted, orig: r, off: (0, 0) });
        } else {
            self.sel_drag = Some(SelDrag::New(t));
            self.sel = None;
        }
        self.refresh();
    }

    fn sel_update(&mut self, t: (i32, i32)) {
        match &mut self.sel_drag {
            Some(SelDrag::New(s)) => {
                let (s, t) = (clamp_t(*s), clamp_t(t));
                let (x0, y0, x1, y1) = (s.0.min(t.0), s.1.min(t.1), s.0.max(t.0), s.1.max(t.1));
                self.sel = Some([x0 as u32, y0 as u32, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32]);
            }
            Some(SelDrag::Move { start, off, .. }) => *off = (t.0 - start.0, t.1 - start.1),
            None => {}
        }
        self.refresh();
    }

    fn sel_end(&mut self) {
        if let Some(SelDrag::Move { lifted, orig, off, .. }) = self.sel_drag.take() {
            let (x, y) = (orig[0] as i32 + off.0, orig[1] as i32 + off.1);
            tools::stamp(&mut self.skin, &lifted, x, y);
            let (cx, cy) = (x.clamp(0, 63) as u32, y.clamp(0, 63) as u32);
            self.sel = Some([cx, cy, orig[2].min(64 - cx), orig[3].min(64 - cy)]);
        }
        self.refresh();
    }
}

fn clamp_t(t: (i32, i32)) -> (i32, i32) {
    (t.0.clamp(0, 63), t.1.clamp(0, 63))
}

fn flood(img: &mut RgbaImage, x: u32, y: u32, [rx, ry, rw, rh]: [u32; 4], c: [u8; 4]) {
    let target = img.get_pixel(x, y).0;
    let mut stack = vec![(x, y)];
    while let Some((x, y)) = stack.pop() {
        if x < rx || y < ry || x >= rx + rw || y >= ry + rh || img.get_pixel(x, y).0 != target {
            continue;
        }
        img.put_pixel(x, y, Rgba(c));
        stack.extend([(x + 1, y), (x.wrapping_sub(1), y), (x, y + 1), (x, y.wrapping_sub(1))]);
    }
}

/// Bresenham line, inclusive of both ends.
fn line(mut x0: i32, mut y0: i32, x1: i32, y1: i32) -> Vec<(i32, i32)> {
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = ((x1 - x0).signum(), (y1 - y0).signum());
    let mut err = dx + dy;
    let mut out = vec![];
    loop {
        out.push((x0, y0));
        if x0 == x1 && y0 == y1 {
            return out;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

pub fn surface(img: &RgbaImage) -> cairo::ImageSurface {
    let (w, h) = img.dimensions();
    let mut d = Vec::with_capacity((w * h * 4) as usize);
    for p in img.pixels() {
        let m = |c: u8| (c as u32 * p[3] as u32 / 255) as u8;
        d.extend([m(p[2]), m(p[1]), m(p[0]), p[3]]);
    }
    cairo::ImageSurface::create_for_data(d, cairo::Format::ARgb32, w as i32, h as i32, w as i32 * 4).unwrap()
}

pub fn rgba(c: [u8; 4]) -> gdk::RGBA {
    gdk::RGBA::new(c[0] as f32 / 255., c[1] as f32 / 255., c[2] as f32 / 255., c[3] as f32 / 255.)
}

pub fn from_rgba(c: &gdk::RGBA) -> [u8; 4] {
    [c.red(), c.green(), c.blue(), c.alpha()].map(|v| (v * 255.).round() as u8)
}

fn set_src(cr: &cairo::Context, c: [u8; 4]) {
    cr.set_source_rgba(c[0] as f64 / 255., c[1] as f64 / 255., c[2] as f64 / 255., c[3] as f64 / 255.);
}

/// (texel size, x offset, y offset) of the 2D canvas.
fn layout2d(w: i32, h: i32, zoom: f64, pan: (f64, f64)) -> (f64, f64, f64) {
    let fit = ((w.min(h) as f64 - 16.) / 64.).floor().max(1.);
    let s = (fit * zoom).floor().max(1.);
    (s, ((w as f64 - 64. * s) / 2.).floor() + pan.0, ((h as f64 - 64. * s) / 2.).floor() + pan.1)
}

/// Text for the status bar: "Head · front (outer layer) · x 3, y 5".
fn describe(slim: bool, x: u32, y: u32) -> String {
    match skin::face_info(slim, x, y) {
        Some((p, ov, f, lx, ly)) => {
            let part = ["part_head", "part_body", "part_rarm", "part_larm", "part_rleg", "part_lleg"][p];
            let face = ["face_right", "face_front", "face_left", "face_back", "face_top", "face_bottom"][f];
            format!("{} · {} ({}) · x {lx}, y {ly}   [{x}, {y}]", t(part), t(face), t(if ov { "overlay" } else { "base" }))
        }
        None => format!("{} [{x}, {y}]", t("unused_area")),
    }
}

/// Pen pressure and the pen's eraser end (tablet mode).
fn stylus(st: &St, area: &gtk::DrawingArea) {
    let g = gtk::GestureStylus::new();
    let s2 = st.clone();
    let update = move |g: &gtk::GestureStylus| {
        let mut s = s2.borrow_mut();
        s.pressure = g.axis(gdk::AxisUse::Pressure).unwrap_or(1.).clamp(0.05, 1.);
        s.stylus_eraser = g.device_tool().is_some_and(|t| t.tool_type() == gdk::DeviceToolType::Eraser);
    };
    let u = update.clone();
    g.connect_down(move |g, _, _| u(g));
    g.connect_motion(move |g, _, _| update(g));
    let s2 = st.clone();
    g.connect_up(move |_, _, _| {
        let mut s = s2.borrow_mut();
        s.pressure = 1.;
        s.stylus_eraser = false;
    });
    area.add_controller(g);
}

fn canvas_2d(st: &St, color_btn: &Picker, hover: &gtk::Label) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
    area.add_css_class("sv-canvas");
    let s2 = st.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let s = s2.borrow();
        let (sc, ox, oy) = layout2d(w, h, s.zoom, s.pan);
        cr.save().ok();
        cr.translate(ox, oy);
        if s.settings.checker {
            for y in 0..64 {
                for x in 0..64 {
                    let v = if (x + y) % 2 == 0 { 0.32 } else { 0.24 };
                    cr.set_source_rgb(v, v, v);
                    cr.rectangle(x as f64 * sc, y as f64 * sc, sc, sc);
                    let _ = cr.fill();
                }
            }
        }
        // Faint outline of every UV face, so you can see which part you are on.
        cr.set_source_rgba(1., 1., 1., 0.07);
        for p in skin::parts(s.slim) {
            for ov in [false, true] {
                for f in skin::part_faces(&p, ov) {
                    let [x, y, fw, fh] = f.rect.map(|v| v as f64 * sc);
                    cr.rectangle(x, y, fw, fh);
                    let _ = cr.fill();
                }
            }
        }
        cr.save().ok();
        cr.scale(sc, sc);
        let _ = cr.set_source_surface(surface(&s.skin), 0., 0.);
        cr.source().set_filter(cairo::Filter::Nearest);
        let _ = cr.paint();
        if let Some(SelDrag::Move { lifted, orig, off, .. }) = &s.sel_drag {
            let _ = cr.set_source_surface(surface(lifted), orig[0] as f64 + off.0 as f64, orig[1] as f64 + off.1 as f64);
            cr.source().set_filter(cairo::Filter::Nearest);
            let _ = cr.paint();
        }
        cr.restore().ok();
        if s.settings.show_grid && sc >= 4. {
            set_src(cr, parse_hex(&s.settings.grid_color));
            cr.set_line_width(1.);
            for i in 0..=64 {
                let v = (i as f64 * sc).round() + 0.5;
                cr.move_to(v, 0.);
                cr.line_to(v, 64. * sc);
                cr.move_to(0., v);
                cr.line_to(64. * sc, v);
            }
            let _ = cr.stroke();
        }
        let sel = match &s.sel_drag {
            Some(SelDrag::Move { orig, off, .. }) => Some([orig[0] as f64 + off.0 as f64, orig[1] as f64 + off.1 as f64, orig[2] as f64, orig[3] as f64]),
            _ => s.sel.map(|r| r.map(|v| v as f64)),
        };
        if let Some([x, y, w, h]) = sel {
            cr.rectangle(x * sc + 0.5, y * sc + 0.5, w * sc - 1., h * sc - 1.);
            cr.set_line_width(2.);
            cr.set_source_rgb(1., 1., 1.);
            cr.set_dash(&[6., 4.], 0.);
            let _ = cr.stroke_preserve();
            cr.set_source_rgb(0., 0., 0.);
            cr.set_dash(&[6., 4.], 5.);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.);
        }
        cr.restore().ok();
        // Minimap while zoomed in.
        if s.zoom > 1. {
            let m = 96.;
            let (mx, my) = (w as f64 - m - 12., 12.);
            cr.set_source_rgba(0., 0., 0., 0.55);
            cr.rectangle(mx - 4., my - 4., m + 8., m + 8.);
            let _ = cr.fill();
            cr.save().ok();
            cr.translate(mx, my);
            cr.scale(m / 64., m / 64.);
            let _ = cr.set_source_surface(surface(&s.skin), 0., 0.);
            cr.source().set_filter(cairo::Filter::Nearest);
            let _ = cr.paint();
            cr.restore().ok();
            let k = m / (64. * sc);
            cr.rectangle(mx + (-ox).max(0.) * k, my + (-oy).max(0.) * k, (w as f64 * k).min(m), (h as f64 * k).min(m));
            cr.set_source_rgb(1., 1., 1.);
            cr.set_line_width(1.5);
            let _ = cr.stroke();
        }
    });

    let texel = |s: &State, area: &gtk::DrawingArea, x: f64, y: f64| {
        let (sc, ox, oy) = layout2d(area.width(), area.height(), s.zoom, s.pan);
        (((x - ox) / sc).floor() as i32, ((y - oy) / sc).floor() as i32)
    };
    let drag = gtk::GestureDrag::builder().button(0).build();
    let (s2, a2, b2) = (st.clone(), area.clone(), color_btn.clone());
    drag.connect_drag_begin(move |g, x, y| {
        if g.current_button() == 2 {
            return;
        }
        let picked = {
            let mut s = s2.borrow_mut();
            let (tx, ty) = texel(&s, &a2, x, y);
            let inside = (0..64).contains(&tx) && (0..64).contains(&ty);
            if g.current_button() == 3 && s.settings.right_click_picks {
                let tool = std::mem::replace(&mut s.tool, Tool::Picker);
                let r = inside.then(|| s.apply(tx as u32, ty as u32)).flatten();
                s.tool = tool;
                r
            } else if s.tool == Tool::Select {
                s.sel_begin((tx, ty));
                None
            } else {
                s.begin();
                if matches!(s.tool_now(), Tool::Pencil | Tool::Eraser) {
                    s.paint_line(tx, ty);
                    None
                } else if inside {
                    s.apply(tx as u32, ty as u32)
                } else {
                    None
                }
            }
        };
        if let Some(c) = picked {
            b2.set_rgba(&rgba(c));
        }
    });
    let (s2, a2) = (st.clone(), area.clone());
    drag.connect_drag_update(move |g, dx, dy| {
        let Some((x, y)) = g.start_point() else { return };
        let mut s = s2.borrow_mut();
        let t_ = texel(&s, &a2, x + dx, y + dy);
        match g.current_button() {
            1 if s.tool == Tool::Select => s.sel_update(t_),
            1 if matches!(s.tool_now(), Tool::Pencil | Tool::Eraser) => s.paint_line(t_.0, t_.1),
            _ => {}
        }
    });
    let s2 = st.clone();
    drag.connect_drag_end(move |g, _, _| {
        let mut s = s2.borrow_mut();
        if g.current_button() == 1 && s.tool == Tool::Select {
            s.sel_end();
        }
    });
    area.add_controller(drag);

    // Middle-drag pans, scroll zooms around the cursor.
    let pan = gtk::GestureDrag::builder().button(2).build();
    let (s2, start) = (st.clone(), Rc::new(std::cell::Cell::new((0., 0.))));
    let st0 = start.clone();
    pan.connect_drag_begin(move |_, _, _| st0.set(s2.borrow().pan));
    let s2 = st.clone();
    pan.connect_drag_update(move |_, dx, dy| {
        let mut s = s2.borrow_mut();
        let (px, py) = start.get();
        s.pan = (px + dx, py + dy);
        s.refresh();
    });
    area.add_controller(pan);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let (s2, a2) = (st.clone(), area.clone());
    let mouse = Rc::new(std::cell::Cell::new((0., 0.)));
    let m2 = mouse.clone();
    scroll.connect_scroll(move |_, _, dy| {
        let mut s = s2.borrow_mut();
        let (w, h) = (a2.width(), a2.height());
        let (mx, my) = m2.get();
        let (sc, ox, oy) = layout2d(w, h, s.zoom, s.pan);
        let (tx, ty) = ((mx - ox) / sc, (my - oy) / sc);
        s.zoom = (s.zoom * 1.25f64.powf(-dy)).clamp(1., 16.);
        if s.zoom <= 1.001 {
            (s.zoom, s.pan) = (1., (0., 0.));
        } else {
            let (sc2, ox0, oy0) = layout2d(w, h, s.zoom, (0., 0.));
            s.pan = (mx - tx * sc2 - ox0, my - ty * sc2 - oy0);
        }
        s.refresh();
        glib::Propagation::Stop
    });
    area.add_controller(scroll);

    let motion = gtk::EventControllerMotion::new();
    let (s2, a2, hv) = (st.clone(), area.clone(), hover.clone());
    motion.connect_motion(move |_, x, y| {
        mouse.set((x, y));
        let s = s2.borrow();
        let (tx, ty) = texel(&s, &a2, x, y);
        hv.set_label(&if (0..64).contains(&tx) && (0..64).contains(&ty) { describe(s.slim, tx as u32, ty as u32) } else { String::new() });
    });
    area.add_controller(motion);
    stylus(st, &area);
    area
}

fn view_3d(st: &St, color_btn: &Picker, hover: &gtk::Label) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder().hexpand(true).vexpand(true).build();
    area.add_css_class("sv-view3d");
    let s2 = st.clone();
    area.set_draw_func(move |_, cr, w, h| {
        let mut s = s2.borrow_mut();
        let q = s.settings.quality.clamp(0.25, 2.);
        let (rw, rh) = (((w as f64 * q) as usize).max(1), ((h as f64 * q) as usize).max(1));
        let mut fr = s.render_view(rw, rh, s.yaw, s.anim_t, None, false);
        // Before / after: the left part of the view shows the skin as it was opened or last saved.
        if let Some(split) = s.before_after {
            let old = s.render_view(rw, rh, s.yaw, s.anim_t, Some(&s.before), false);
            let cut = ((rw as f32 * split) as usize).min(rw);
            for y in 0..rh {
                let row = y * rw * 4;
                fr.color[row..row + cut * 4].copy_from_slice(&old.color[row..row + cut * 4]);
                if cut < rw {
                    fr.color[row + cut * 4..row + cut * 4 + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
        let data = std::mem::take(&mut fr.color);
        if let Ok(surf) = cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, rw as i32, rh as i32, rw as i32 * 4) {
            cr.scale(1. / q, 1. / q);
            let _ = cr.set_source_surface(&surf, 0., 0.);
            cr.source().set_filter(if q < 1. { cairo::Filter::Nearest } else { cairo::Filter::Good });
            let _ = cr.paint();
        }
        s.frame = Some(fr);
        s.frame_q = q;
    });

    let s2 = st.clone();
    area.add_tick_callback(move |a, clock| {
        let now = clock.frame_time() as f64 / 1e6;
        let mut s = s2.borrow_mut();
        let dt = (now - s.last_tick).clamp(0., 0.1) as f32;
        s.last_tick = now;
        if s.spin && s.drag_rot.is_none() {
            s.yaw += dt * s.settings.rotate_speed as f32;
        }
        if s.walk {
            s.anim_t += dt * 5. * s.settings.walk_speed as f32;
        }
        s.clock += dt;
        let emoting = s.emote.is_some_and(|(_, t0)| s.clock - t0 < 1.9);
        if !emoting {
            s.emote = None;
        }
        if s.spin || s.walk || emoting {
            a.queue_draw();
        }
        glib::ControlFlow::Continue
    });

    let drag = gtk::GestureDrag::builder().button(0).build();
    let (s2, b2) = (st.clone(), color_btn.clone());
    drag.connect_drag_begin(move |g, x, y| {
        let picked = {
            let mut s = s2.borrow_mut();
            if g.current_button() == 1 && s.paint3d && s.tool != Tool::Select {
                s.begin();
                s.paint_3d(x, y)
            } else {
                s.drag_rot = Some((s.yaw, s.pitch));
                None
            }
        };
        if let Some(c) = picked {
            b2.set_rgba(&rgba(c));
        }
    });
    let (s2, a2) = (st.clone(), area.clone());
    drag.connect_drag_update(move |g, dx, dy| {
        let mut s = s2.borrow_mut();
        if let Some((yaw, pitch)) = s.drag_rot {
            s.yaw = yaw + dx as f32 * 0.01;
            s.pitch = (pitch + dy as f32 * 0.01).clamp(-1.5, 1.5);
            a2.queue_draw();
        } else if matches!(s.tool_now(), Tool::Pencil | Tool::Eraser) {
            if let Some((x, y)) = g.start_point() {
                s.paint_3d(x + dx, y + dy);
            }
        }
    });
    let s2 = st.clone();
    drag.connect_drag_end(move |_, _, _| s2.borrow_mut().drag_rot = None);
    area.add_controller(drag);

    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let (s2, a2) = (st.clone(), area.clone());
    scroll.connect_scroll(move |_, _, dy| {
        let mut s = s2.borrow_mut();
        s.dist = (s.dist * 1.1f32.powf(dy as f32)).clamp(20., 200.);
        a2.queue_draw();
        glib::Propagation::Stop
    });
    area.add_controller(scroll);

    let motion = gtk::EventControllerMotion::new();
    let (s2, hv) = (st.clone(), hover.clone());
    motion.connect_motion(move |_, x, y| {
        let s = s2.borrow();
        hv.set_label(&s.pick_3d(x, y).map(|(u, v, _)| describe(s.slim, u, v)).unwrap_or_default());
    });
    area.add_controller(motion);
    stylus(st, &area);
    area
}

fn palette(st: &St, color_btn: &Picker) -> gtk::DrawingArea {
    // Two rows of 12: the 16 dye colours, then the 8 most recent colours. Swatch size is a setting.
    const COLS: usize = 12;
    let n = PALETTE.len() + RECENT;
    let sw = |s: &State| s.settings.palette_size.clamp(12., 48.);
    let size = |sw: f64| (((COLS as f64) * (sw + 3.)) as i32, (2. * (sw + 3.)) as i32);
    let (w0, h0) = size(sw(&st.borrow()));
    let area = gtk::DrawingArea::builder().content_width(w0).content_height(h0).valign(gtk::Align::Center).build();
    let colors = |s: &State| -> Vec<Option<[u8; 4]>> {
        PALETTE.iter().map(|c| Some([(c >> 16) as u8, (c >> 8) as u8, *c as u8, 255])).chain((0..RECENT).map(|i| s.recent.get(i).copied())).collect()
    };
    let s2 = st.clone();
    area.set_draw_func(move |a, cr, _, _| {
        let s = s2.borrow();
        let swz = sw(&s);
        let step = swz + 3.;
        let (w, h) = size(swz);
        if a.content_width() != w || a.content_height() != h {
            let a = a.clone();
            glib::idle_add_local_once(move || {
                a.set_content_width(w);
                a.set_content_height(h);
            });
        }
        for (i, c) in colors(&s).into_iter().enumerate().take(n) {
            let (x, y) = ((i % COLS) as f64 * step, (i / COLS) as f64 * step);
            cr.new_sub_path();
            cr.arc(x + swz / 2., y + swz / 2., swz / 2. - 1., 0., std::f64::consts::TAU);
            match c {
                Some(c) => {
                    set_src(cr, c);
                    let _ = cr.fill_preserve();
                    cr.set_source_rgba(0., 0., 0., 0.3);
                }
                None => cr.set_source_rgba(0.5, 0.5, 0.5, 0.3),
            }
            cr.set_line_width(1.);
            let _ = cr.stroke();
        }
    });
    let click = gtk::GestureClick::new();
    let (s2, b2) = (st.clone(), color_btn.clone());
    click.connect_pressed(move |_, _, x, y| {
        let step = sw(&s2.borrow()) + 3.;
        let i = (y / step) as usize * COLS + (x / step) as usize;
        let c = colors(&s2.borrow()).get(i).copied().flatten();
        if let Some(c) = c {
            s2.borrow_mut().color = c;
            b2.set_rgba(&rgba(c));
        }
    });
    area.add_controller(click);
    area
}

fn toggle(label: &str, tip: Option<&str>, active: bool, st: &St, f: impl Fn(&mut State, bool) + 'static) -> gtk::ToggleButton {
    let b = gtk::ToggleButton::builder().label(label).active(active).build();
    b.set_tooltip_text(tip);
    let st = st.clone();
    b.connect_toggled(move |b| {
        let mut s = st.borrow_mut();
        f(&mut s, b.is_active());
        s.refresh();
    });
    b
}

fn check(label: &str, active: bool, st: &St, f: impl Fn(&mut State, bool) + 'static) -> gtk::CheckButton {
    let b = gtk::CheckButton::builder().label(label).active(active).build();
    let st = st.clone();
    b.connect_toggled(move |b| {
        let mut s = st.borrow_mut();
        f(&mut s, b.is_active());
        s.refresh();
    });
    b
}

/// Colour schemes built from the current colour; click a swatch to use it.
fn harmony_button(st: &St, color_btn: &Picker) -> gtk::MenuButton {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    list.set_margin_start(8);
    list.set_margin_end(8);
    list.set_margin_top(8);
    list.set_margin_bottom(8);
    let pop = gtk::Popover::builder().child(&list).build();
    let (s2, l2, b2, p2) = (st.clone(), list.clone(), color_btn.clone(), pop.clone());
    pop.connect_show(move |_| {
        while let Some(c) = l2.first_child() {
            l2.remove(&c);
        }
        let base = s2.borrow().color;
        for (i, key) in tools::HARMONIES.iter().enumerate() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            row.append(&gtk::Label::builder().label(t(key)).xalign(0.).width_chars(16).build());
            for c in tools::harmony(base, i) {
                let sw = gtk::DrawingArea::builder().content_width(26).content_height(26).build();
                sw.set_draw_func(move |_, cr, w, h| {
                    cr.arc(w as f64 / 2., h as f64 / 2., w as f64 / 2. - 1., 0., std::f64::consts::TAU);
                    set_src(cr, c);
                    let _ = cr.fill();
                });
                let click = gtk::GestureClick::new();
                let (s3, b3, p3) = (s2.clone(), b2.clone(), p2.clone());
                click.connect_released(move |_, _, _, _| {
                    s3.borrow_mut().color = c;
                    b3.set_rgba(&rgba(c));
                    p3.popdown();
                });
                sw.add_controller(click);
                row.append(&sw);
            }
            l2.append(&row);
        }
    });
    gtk::MenuButton::builder().icon_name("preferences-color-symbolic").tooltip_text(t("harmony")).popover(&pop).valign(gtk::Align::Center).build()
}

fn icon_btn(icon: &str, tip: &str, action: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.set_tooltip_text(Some(tip));
    b.set_action_name(Some(action));
    b
}

fn recent_button(st: &St) -> gtk::MenuButton {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let pop = gtk::Popover::builder().child(&list).build();
    let s2 = st.clone();
    let (l2, p2) = (list.clone(), pop.clone());
    pop.connect_show(move |_| {
        while let Some(c) = l2.first_child() {
            l2.remove(&c);
        }
        let recent = s2.borrow().settings.recent.clone();
        if recent.is_empty() {
            l2.append(&gtk::Label::builder().label(t("no_recent")).css_classes(["dim-label"]).margin_start(8).margin_end(8).margin_top(8).margin_bottom(8).build());
        }
        for path in recent {
            let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let b = gtk::Button::builder().label(name).tooltip_text(&path).css_classes(["flat"]).build();
            b.set_action_name(Some("app.open-path"));
            b.set_action_target_value(Some(&path.to_variant()));
            let p3 = p2.clone();
            b.connect_clicked(move |_| p3.popdown());
            l2.append(&b);
        }
    });
    gtk::MenuButton::builder().icon_name("document-open-recent-symbolic").tooltip_text(t("recent")).popover(&pop).build()
}

fn tools_menu() -> gtk::MenuButton {
    let m = gio::Menu::new();
    let tools = gio::Menu::new();
    tools.append(Some(t("adjust_colors")), Some("app.adjust"));
    tools.append(Some(t("reduce_colors")), Some("app.quantize"));
    tools.append(Some(t("transfer_parts")), Some("app.transfer"));
    tools.append(Some(t("random_skin")), Some("app.random"));
    tools.append(Some(t("check_skin")), Some("app.check"));
    tools.append(Some(t("from_picture")), Some("app.picture"));
    tools.append(Some(t("timelapse")), Some("app.timelapse"));
    m.append_section(None, &tools);
    let sel = gio::Menu::new();
    sel.append(Some(t("sel_copy")), Some("app.sel-copy"));
    sel.append(Some(t("sel_paste")), Some("app.sel-paste"));
    sel.append(Some(t("sel_rotate")), Some("app.sel-rotate"));
    sel.append(Some(t("sel_flip")), Some("app.sel-flip"));
    sel.append(Some(t("sel_delete")), Some("app.sel-delete"));
    m.append_section(Some(t("selection")), &sel);
    let share = gio::Menu::new();
    share.append(Some(t("mineskin_upload")), Some("app.mineskin"));
    m.append_section(None, &share);
    // Plugins: refilled every time the menu opens, so new .lua files show up without a restart.
    let plugins = gio::Menu::new();
    m.append_section(Some(t("plugins")), &plugins);
    let fill = move || {
        plugins.remove_all();
        for (path, name) in crate::plugins::list() {
            let item = gio::MenuItem::new(Some(&name), None);
            item.set_action_and_target_value(Some("app.plugin"), Some(&path.to_string_lossy().to_variant()));
            plugins.append_item(&item);
        }
        plugins.append(Some(t("plugins_folder")), Some("app.plugins-folder"));
    };
    fill();
    let b = gtk::MenuButton::builder().icon_name("open-menu-symbolic").tooltip_text(t("more_tools")).menu_model(&m).build();
    b.connect_active_notify(move |b| {
        if b.is_active() {
            fill();
        }
    });
    b
}

fn dropdown(items: &[&str], sel: u32, st: &St, f: impl Fn(&mut State, u32) + 'static) -> gtk::DropDown {
    let d = gtk::DropDown::from_strings(items);
    d.set_selected(sel);
    let st = st.clone();
    d.connect_selected_notify(move |d| {
        let mut s = st.borrow_mut();
        f(&mut s, d.selected());
        s.refresh();
    });
    d
}

fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder().label(text).xalign(0.).css_classes(["heading"]).margin_top(6).build()
}

/// Second column of the 3D menu: scene, camera presets, props, comparison, recording.
fn view_extras(st: &St) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let s = st.borrow();
    b.append(&heading(t("scene")));
    b.append(&dropdown(&[t("scene_studio"), t("scene_game"), t("scene_day"), t("scene_night")], s.scene, st, |s, v| {
        s.scene = v;
        if v == 1 {
            // Third-person camera from behind, like F5 in game.
            (s.yaw, s.pitch, s.dist) = (std::f32::consts::PI, 0.35, 75.);
        }
    }));
    b.append(&heading(t("angles")));
    let angles = gtk::Box::builder().css_classes(["linked"]).build();
    let pi = std::f32::consts::PI;
    for (key, yaw, pitch) in [("view_front", 0., 0.1), ("view_back", pi, 0.1), ("view_left", -pi / 2., 0.1), ("view_right", pi / 2., 0.1), ("view_top", 0., 1.45)] {
        let btn = gtk::Button::with_label(t(key));
        let s2 = st.clone();
        btn.connect_clicked(move |_| s2.borrow_mut().set_view(yaw, pitch));
        angles.append(&btn);
    }
    b.append(&angles);
    b.append(&heading(t("explode")));
    let ex = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0., 1., 0.05);
    ex.set_value(s.explode as f64);
    let s2 = st.clone();
    ex.connect_value_changed(move |sc| {
        let mut s = s2.borrow_mut();
        s.explode = sc.value() as f32;
        s.refresh();
    });
    b.append(&ex);
    b.append(&heading(t("props")));
    let items: Vec<&str> = tools::ITEMS.iter().map(|k| t(k)).collect();
    b.append(&dropdown(&items, s.item as u32, st, |s, v| s.item = v as usize));
    let armors: Vec<&str> = tools::ARMORS.iter().map(|a| t(a.0)).collect();
    b.append(&dropdown(&armors, s.armor as u32, st, |s, v| s.armor = v as usize));
    b.append(&heading(t("compare")));
    // Tab list changes, so the choices are rebuilt each time the menu opens.
    let cmp = gtk::DropDown::from_strings(&[t("compare_none")]);
    let s2 = st.clone();
    cmp.connect_selected_notify(move |d| {
        if let Ok(mut s) = s2.try_borrow_mut() {
            let i = d.selected() as usize;
            let others: Vec<usize> = (0..s.tabs.len()).filter(|&j| j != s.active).collect();
            s.compare = if i == 0 { None } else { others.get(i - 1).copied() };
            s.refresh();
        }
    });
    let (s2, c2) = (st.clone(), cmp.clone());
    b.connect_map(move |_| {
        let s = s2.borrow();
        let titles = s.tabs();
        let mut names = vec![t("compare_none").to_string()];
        let others: Vec<usize> = (0..titles.len()).filter(|&j| j != s.active).collect();
        names.extend(others.iter().map(|&j| titles[j].clone()));
        let sel = s.compare.and_then(|c| others.iter().position(|&j| j == c)).map(|p| p as u32 + 1).unwrap_or(0);
        drop(s);
        let refs: Vec<&str> = names.iter().map(|n| n.as_str()).collect();
        let c3 = c2.clone();
        let model = gtk::StringList::new(&refs);
        glib::idle_add_local_once(move || {
            c3.set_model(Some(&model));
            c3.set_selected(sel);
        });
    });
    b.append(&cmp);
    let ba = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0., 1., 0.01);
    ba.set_value(0.5);
    ba.set_sensitive(s.before_after.is_some());
    let (s2, ba2) = (st.clone(), ba.clone());
    let ba_check = gtk::CheckButton::builder().label(t("before_after")).active(s.before_after.is_some()).tooltip_text(t("before_after_tip")).build();
    ba_check.connect_toggled(move |c| {
        let mut s = s2.borrow_mut();
        s.before_after = c.is_active().then(|| ba2.value() as f32);
        ba2.set_sensitive(c.is_active());
        s.refresh();
    });
    let s2 = st.clone();
    ba.connect_value_changed(move |sc| {
        let mut s = s2.borrow_mut();
        if s.before_after.is_some() {
            s.before_after = Some(sc.value() as f32);
            s.refresh();
        }
    });
    b.append(&ba_check);
    b.append(&ba);
    b.append(&heading(t("record")));
    let rec = gtk::Box::builder().css_classes(["linked"]).build();
    for (key, action) in [("save_gif", "app.gif"), ("save_frame", "app.frame")] {
        let btn = gtk::Button::with_label(t(key));
        btn.set_action_name(Some(action));
        rec.append(&btn);
    }
    b.append(&rec);
    b
}

/// Tab strip; rebuilt only when titles or the active tab change.
fn tab_strip(st: &St) -> gtk::Box {
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    strip.add_css_class("sv-tabs");
    let sig: Rc<RefCell<String>> = Rc::default();
    let fill = {
        let (strip, sig, st) = (strip.clone(), sig.clone(), st.clone());
        move |s: &State| {
            let titles = s.tabs();
            let now = format!("{}|{}", s.active_tab(), titles.join("|"));
            if *sig.borrow() == now {
                return;
            }
            *sig.borrow_mut() = now;
            while let Some(c) = strip.first_child() {
                strip.remove(&c);
            }
            for (i, title) in titles.iter().enumerate() {
                let tab = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                tab.add_css_class("sv-tab");
                if i == s.active_tab() {
                    tab.add_css_class("active");
                }
                let b = gtk::Button::builder().label(title).css_classes(["flat"]).build();
                let s2 = st.clone();
                // Deferred: we're inside a listener while the state is borrowed.
                b.connect_clicked(move |_| {
                    let s3 = s2.clone();
                    glib::idle_add_local_once(move || s3.borrow_mut().switch_tab(i));
                });
                let x = gtk::Button::builder().icon_name("window-close-symbolic").css_classes(["flat", "circular"]).tooltip_text(t("close_tab")).build();
                x.set_action_name(Some("app.close-tab"));
                x.set_action_target_value(Some(&(i as u32).to_variant()));
                tab.append(&b);
                tab.append(&x);
                strip.append(&tab);
            }
            let plus = gtk::Button::builder().icon_name("tab-new-symbolic").css_classes(["flat"]).tooltip_text(t("new_tab")).build();
            plus.set_action_name(Some("app.new"));
            strip.append(&plus);
        }
    };
    fill(&st.borrow());
    st.borrow_mut().listeners.push(Box::new(fill));
    strip
}

pub fn page(st: &St) -> gtk::Widget {
    let (s2, s3) = (st.clone(), st.clone());
    let color_btn = Picker::new(st.borrow().color, move || s3.borrow().settings.picker_size as i32, move |c| s2.borrow_mut().color = c);
    color_btn.button.set_tooltip_text(Some(t("color")));

    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    bar.add_css_class("sv-toolbar");
    let file = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    file.add_css_class("linked");
    file.append(&icon_btn("document-new-symbolic", t("new"), "app.new"));
    file.append(&icon_btn("document-open-symbolic", t("open"), "app.open"));
    file.append(&recent_button(st));
    file.append(&icon_btn("document-save-symbolic", t("save"), "app.save"));
    file.append(&icon_btn("document-save-as-symbolic", t("save_as"), "app.save-as"));
    file.append(&icon_btn("edit-paste-symbolic", t("paste"), "app.paste"));
    bar.append(&file);
    let hist = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    hist.add_css_class("linked");
    hist.append(&icon_btn("edit-undo-symbolic", t("undo"), "app.undo"));
    hist.append(&icon_btn("edit-redo-symbolic", t("redo"), "app.redo"));
    bar.append(&hist);

    let tools_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tools_box.add_css_class("linked");
    let mut first: Option<gtk::ToggleButton> = None;
    let cur = st.borrow().tool;
    for (tool, icon, tip) in [
        (Tool::Pencil, "document-edit-symbolic", "pencil"),
        (Tool::Eraser, "edit-clear-symbolic", "eraser"),
        (Tool::Fill, "format-justify-fill-symbolic", "fill"),
        (Tool::Picker, "color-select-symbolic", "picker"),
        (Tool::Replace, "edit-find-replace-symbolic", "replace_tool"),
        (Tool::Select, "edit-select-symbolic", "select_tool"),
    ] {
        let b = gtk::ToggleButton::builder().icon_name(icon).tooltip_text(t(tip)).active(tool == cur).build();
        b.set_group(first.as_ref());
        first.get_or_insert(b.clone());
        let s2 = st.clone();
        b.connect_toggled(move |b| {
            if b.is_active() {
                let mut s = s2.borrow_mut();
                s.tool = tool;
                if tool != Tool::Select {
                    s.sel = None;
                }
                s.refresh();
            }
        });
        tools_box.append(&b);
    }
    bar.append(&tools_box);

    // Brush size + shape.
    let size = gtk::SpinButton::with_range(1., 8., 1.);
    size.set_value(st.borrow().brush as f64);
    size.set_tooltip_text(Some(t("brush_size")));
    size.set_valign(gtk::Align::Center);
    let s2 = st.clone();
    size.connect_value_changed(move |b| s2.borrow_mut().brush = b.value() as u32);
    let round = gtk::ToggleButton::builder().icon_name("media-record-symbolic").tooltip_text(t("brush_round")).active(st.borrow().round).valign(gtk::Align::Center).build();
    let s2 = st.clone();
    round.connect_toggled(move |b| s2.borrow_mut().round = b.is_active());
    let brush = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    brush.add_css_class("linked");
    brush.append(&size);
    brush.append(&round);
    bar.append(&brush);
    bar.append(&color_btn.button);
    bar.append(&harmony_button(st, &color_btn));
    let pal = palette(st, &color_btn);
    bar.append(&pal);
    bar.append(&tools_menu());

    let hover = gtk::Label::builder().xalign(0.).css_classes(["dim-label", "caption"]).build();
    let c2d = canvas_2d(st, &color_btn, &hover);
    let v3d = view_3d(st, &color_btn, &hover);
    let (a, b, p) = (c2d.clone(), v3d.clone(), pal.clone());
    st.borrow_mut().listeners.push(Box::new(move |_| {
        a.queue_draw();
        b.queue_draw();
        p.queue_draw();
    }));

    let s = st.borrow();
    // Floating bar over the 3D view: main toggles + a menu with the rest.
    let bar3d = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bar3d.add_css_class("sv-floatbar");
    bar3d.set_halign(gtk::Align::Center);
    bar3d.set_valign(gtk::Align::End);
    bar3d.set_margin_bottom(12);
    bar3d.append(&toggle(t("layers3d"), Some(t("layers3d_tip")), s.voxels, st, |s, v| s.voxels = v));
    bar3d.append(&toggle(t("paint3d"), Some(t("paint3d_tip")), s.paint3d, st, |s, v| s.paint3d = v));
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 6);
    menu.set_margin_start(6);
    menu.set_margin_end(6);
    menu.set_margin_top(6);
    menu.set_margin_bottom(6);
    menu.append(&check(t("base"), s.show_base, st, |s, v| s.show_base = v));
    menu.append(&check(t("overlay"), s.show_overlay, st, |s, v| s.show_overlay = v));
    let po = check(t("paint_overlay"), s.paint_overlay, st, |s, v| s.paint_overlay = v);
    po.set_tooltip_text(Some(t("paint_overlay_tip")));
    menu.append(&po);
    menu.append(&check(t("walk"), s.walk, st, |s, v| s.walk = v));
    menu.append(&check(t("rotate"), s.spin, st, |s, v| s.spin = v));
    let model = gtk::DropDown::from_strings(&[t("classic"), t("slim")]);
    model.set_selected(s.slim as u32);
    let s2 = st.clone();
    model.connect_selected_notify(move |d| {
        // Guarded: the listener below changes it programmatically while the state may be busy.
        if let Ok(mut s) = s2.try_borrow_mut() {
            s.slim = d.selected() == 1;
            s.refresh();
        }
    });
    menu.append(&model);
    let reset = gtk::Button::with_label(t("reset_view"));
    let s2 = st.clone();
    reset.connect_clicked(move |_| {
        let mut s = s2.borrow_mut();
        (s.yaw, s.pitch, s.dist) = (0.5, 0.25, 70.);
        s.refresh();
    });
    menu.append(&reset);
    let cols = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    cols.append(&menu);
    cols.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    let extras = view_extras(st);
    extras.set_margin_top(6);
    extras.set_margin_bottom(6);
    extras.set_margin_end(6);
    cols.append(&extras);
    let more = gtk::MenuButton::builder().icon_name("view-more-symbolic").popover(&gtk::Popover::builder().child(&cols).build()).build();
    bar3d.append(&more);
    drop(s);
    // Keep the model dropdown in sync when a loaded skin changes the model type.
    let m2 = model.clone();
    st.borrow_mut().listeners.push(Box::new(move |s| {
        if m2.selected() != s.slim as u32 {
            let m3 = m2.clone();
            let slim = s.slim;
            glib::idle_add_local_once(move || m3.set_selected(slim as u32));
        }
    }));

    let right = gtk::Overlay::builder().child(&v3d).build();
    right.add_overlay(&bar3d);
    let split = gtk::Box::builder().homogeneous(true).vexpand(true).build();
    split.append(&c2d);
    split.append(&right);

    // The toolbar scrolls instead of forcing a minimum window width.
    let bar_scroll = gtk::ScrolledWindow::builder().child(&bar).vscrollbar_policy(gtk::PolicyType::Never).build();
    let tabs_scroll = gtk::ScrolledWindow::builder().child(&tab_strip(st)).vscrollbar_policy(gtk::PolicyType::Never).build();
    let status = gtk::Box::builder().css_classes(["sv-status"]).build();
    status.append(&hover);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&bar_scroll);
    root.append(&tabs_scroll);
    root.append(&split);
    root.append(&status);
    root.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flood_stays_inside_face() {
        let mut img = RgbaImage::new(64, 64);
        flood(&mut img, 9, 9, [8, 8, 8, 8], [1, 2, 3, 255]);
        assert_eq!(img.get_pixel(15, 15).0, [1, 2, 3, 255]);
        assert_eq!(img.get_pixel(16, 8).0, [0; 4]);
        assert_eq!(img.get_pixel(7, 8).0, [0; 4]);
    }

    #[test]
    fn line_is_continuous() {
        let l = line(0, 0, 5, 2);
        assert_eq!(l.first(), Some(&(0, 0)));
        assert_eq!(l.last(), Some(&(5, 2)));
        assert_eq!(l.len(), 6);
    }

    #[test]
    fn undo_snapshot_only_on_change() {
        let mut s = State::new(Settings::default());
        s.begin();
        let c = s.skin.get_pixel(20, 20).0;
        s.color = c;
        s.apply(20, 20);
        assert!(s.undo.is_empty() && !s.dirty);
        s.color = [9, 9, 9, 255];
        s.apply(20, 20);
        s.apply(21, 20);
        assert_eq!(s.undo.len(), 1);
        s.undo();
        assert_eq!(s.skin.get_pixel(20, 20).0, c);
    }

    #[test]
    fn big_brush_stays_in_face() {
        let mut s = State::new(Settings::default());
        s.brush = 5;
        s.color = [9, 9, 9, 255];
        s.begin();
        s.apply(8, 10); // head front, left edge
        assert_eq!(s.skin.get_pixel(10, 10).0, [9, 9, 9, 255]);
        assert_ne!(s.skin.get_pixel(7, 10).0, [9, 9, 9, 255]); // head right side untouched
    }

    #[test]
    fn tabs_keep_their_own_skin() {
        let mut s = State::new(Settings::default());
        let mut a = RgbaImage::new(64, 64);
        a.put_pixel(1, 1, Rgba([1, 0, 0, 255]));
        s.open_doc(a, false, None, false); // reuses the pristine first tab
        assert_eq!(s.tabs().len(), 1);
        s.mark_dirty();
        s.open_doc(skin::default_skin(), false, None, false);
        assert_eq!(s.tabs().len(), 2);
        s.switch_tab(0);
        assert_eq!(s.skin.get_pixel(1, 1).0, [1, 0, 0, 255]);
        assert!(s.any_dirty());
        s.close_tab(0);
        assert_eq!(s.tabs().len(), 1);
        assert!(!s.any_dirty());
    }

    #[test]
    fn emotes_and_timelapse() {
        let mut s = State::new(Settings::default());
        s.emote(0);
        s.clock += 0.4;
        let mut pose = [[0.; 3]; 6];
        s.emote_pose(&mut pose);
        assert!(pose[skin::RARM][2] < -1.5); // waving arm is up
        s.emote(1);
        s.clock += 0.45;
        assert!(s.emote_pose(&mut [[0.; 3]; 6]) > 3.); // mid-jump
        s.clock += 5.;
        assert_eq!(s.emote_pose(&mut [[0.; 3]; 6]), 0.);
        s.color = [1, 2, 3, 255];
        s.begin();
        s.apply(10, 10);
        assert_eq!(s.timelapse().len(), 2);
    }

    #[test]
    fn selection_move_and_rotate() {
        let mut s = State::new(Settings::default());
        s.skin = RgbaImage::new(64, 64);
        s.skin.put_pixel(2, 2, Rgba([5, 5, 5, 255]));
        s.sel = Some([2, 2, 2, 1]);
        s.sel_begin((2, 2));
        s.sel_update((4, 3));
        s.sel_end();
        assert_eq!(s.skin.get_pixel(4, 3).0, [5, 5, 5, 255]);
        assert_eq!(s.skin.get_pixel(2, 2).0, [0; 4]);
        assert_eq!(s.sel, Some([4, 3, 2, 1]));
        s.sel_transform(tools::rotate90);
        assert_eq!(s.sel, Some([4, 3, 1, 2]));
        s.undo();
        s.undo();
        assert_eq!(s.skin.get_pixel(2, 2).0, [5, 5, 5, 255]);
    }

    /// Dev helper: OUT=dir cargo test scene_dump -- --ignored
    #[test]
    #[ignore]
    fn scene_dump() {
        let out = std::env::var("OUT").unwrap();
        let mut s = State::new(Settings::default());
        s.skin = tools::random_skin(7);
        let shots: [(&str, u32, usize, usize, f32); 4] = [("night", 3, 1, 4, 0.), ("game", 1, 2, 2, 0.), ("day", 2, 4, 3, 0.), ("explode", 0, 0, 0, 1.)];
        for (name, scene, item, armor, ex) in shots {
            (s.scene, s.item, s.armor, s.explode) = (scene, item, armor, ex);
            let yaw = if scene == 1 { std::f32::consts::PI - 0.5 } else { 0.6 };
            let fr = s.render_view(400, 400, yaw, 0., None, false);
            crate::dialogs::frame_to_image(&fr.color, 400, 400).save(format!("{out}/{name}.png")).unwrap();
        }
        crate::packs::panorama(&s.skin, false)[0].save(format!("{out}/pano0.png")).unwrap();
        image::imageops::resize(&crate::packs::papercraft(&s.skin, false), 620, 877, image::imageops::FilterType::Triangle).save(format!("{out}/paper.png")).unwrap();
    }
}
