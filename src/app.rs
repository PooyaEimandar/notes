//! The scene itself: what is drawn, how it moves, and how it answers input.
//!
//! The page talks to the scene through `send` and reads label positions with
//! `labels`. The scene answers with the events in `bridge`.

use std::cell::RefCell;

use sib::render::glam::{Vec2, Vec3};
use sib::render::winit::dpi::PhysicalSize;
use sib::render::winit::event::{
    ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};
use sib::render::{Example, ExampleSettings, FrameStats, RenderContext, RenderResult, wgpu};

use crate::bridge;
use crate::data::SceneData;
use crate::gpu::{Globals, Gpu, Instance};
use crate::orbit::{Orbit, View};

const ORB_SIZE: f32 = 1.25;
const DIM_ORB_SIZE: f32 = 0.5;
const STAR_COUNT: usize = 320;
const INTRO_SECONDS: f32 = 1.8;
const LABELS: usize = 12;
const LABELS_WHEN_FILTERING: usize = 32;
const NEVER: f32 = -1.0e6;
/// The shaders' clock wraps at this many seconds, so an f32 still resolves
/// single frames however long the page stays open. Every animation in
/// `scene.wgsl` repeats a whole number of times in this span, so nothing jumps
/// when the clock wraps.
const CLOCK_WRAP: f64 = 3600.0;
/// Links are drawn within this distance of the camera.
const LINK_RANGE: f32 = 90.0;
/// With more notes than this, distant orbs are dimmed and drawn smaller.
const SPARSE_NOTES: f32 = 400.0;
const ACCENT: [f32; 3] = [0.337, 0.910, 1.0];
const STAR_COLOR: [f32; 3] = [0.43, 0.72, 0.79];
/// #010304, the page background.
const CLEAR: [f64; 3] = [1.0 / 255.0, 3.0 / 255.0, 4.0 / 255.0];

const STATE_DIM: f32 = 0.0;
const STATE_LIT: f32 = 1.0;
const STATE_STAR: f32 = 2.0;

/// Values per label returned by `labels`: note, x, y, radius, flags.
pub const LABEL_STRIDE: usize = 5;
pub const FLAG_HOVERED: f32 = 1.0;
pub const FLAG_SELECTED: f32 = 2.0;

pub enum Command {
    LoadScene(Vec<u8>),
    /// The notes that match the search and tags, or `None` when nothing is
    /// being filtered and every note is lit.
    Matches(Option<Vec<u32>>),
    Select(Option<u32>),
    ReducedMotion(bool),
    Paused(bool),
    /// Parts of the canvas covered by the interface: top, right, bottom and
    /// left, in CSS pixels.
    Insets([f32; 4]),
}

thread_local! {
    static COMMANDS: RefCell<Vec<Command>> = const { RefCell::new(Vec::new()) };
    static LABEL_SNAPSHOT: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
}

pub fn send(command: Command) {
    COMMANDS.with(|commands| commands.borrow_mut().push(command));
}

pub fn labels() -> Vec<f32> {
    LABEL_SNAPSHOT.with(|snapshot| snapshot.borrow().clone())
}

/// A small repeatable generator, so the stars are the same on every visit.
struct Random(u32);

impl Random {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 8) as f32 / (1_u32 << 24) as f32
    }

    fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }
}

/// Rounds a speed in turns per second to one that completes a whole number of
/// turns before the shader clock wraps.
fn whole_turns(speed: f32) -> f32 {
    let wrap = CLOCK_WRAP as f32;
    (speed * wrap).round().max(1.0) / wrap
}

#[derive(Default)]
struct Pointer {
    cursor: Option<Vec2>,
    pressed: bool,
    travel: f32,
    touches: Vec<(u64, Vec2)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Cursor {
    Grab,
    Grabbing,
    Pointer,
}

pub struct Notes {
    gpu: Option<Gpu>,
    scene: SceneData,
    lit: Vec<bool>,
    flash_at: Vec<f32>,
    filtering: bool,
    instances_changed: bool,
    orbit: Orbit,
    stats: FrameStats,
    /// Seconds since launch. An f32 here would stop resolving single frames
    /// after a few hours and stop advancing altogether after a few days.
    clock: f64,
    intro: f32,
    reduced_motion: bool,
    paused: bool,
    insets: [f32; 4],
    selected: Option<u32>,
    hovered: Option<u32>,
    pointer: Pointer,
    cursor: Cursor,
    srgb: bool,
}

impl Default for Notes {
    fn default() -> Self {
        Self {
            gpu: None,
            scene: SceneData::default(),
            lit: Vec::new(),
            flash_at: Vec::new(),
            filtering: false,
            instances_changed: false,
            orbit: Orbit::default(),
            stats: FrameStats::new(),
            clock: 0.0,
            intro: 0.0,
            reduced_motion: false,
            paused: false,
            insets: [0.0; 4],
            selected: None,
            hovered: None,
            pointer: Pointer::default(),
            cursor: Cursor::Grab,
            srgb: false,
        }
    }
}

impl Notes {
    fn size(context: &RenderContext) -> Vec2 {
        Vec2::new(
            context.surface_config.width.max(1) as f32,
            context.surface_config.height.max(1) as f32,
        )
    }

    fn ratio(context: &RenderContext) -> f32 {
        (context.window.scale_factor() as f32).max(1.0)
    }

    fn reach(&self) -> f32 {
        self.orbit.distance + self.scene.radius.max(12.0) * 8.0
    }

    fn view(&self, context: &RenderContext) -> View {
        self.orbit.view(Self::size(context), self.reach())
    }

    /// Fits the camera to the part of the canvas the interface leaves free.
    fn frame(&mut self, context: &RenderContext) {
        let size = Self::size(context) / Self::ratio(context);
        let [top, right, bottom, left] = self.insets;
        let free = Vec2::new(
            (size.x - left - right).max(size.x * 0.3),
            (size.y - top - bottom).max(size.y * 0.3),
        );
        self.orbit.frame(
            self.scene.centroid,
            self.scene.radius * size.y / free.y,
            free.x / free.y,
        );
        self.orbit
            .set_offset(Vec2::new((left - right) / size.x, (bottom - top) / size.y));
        match self
            .selected
            .and_then(|id| self.scene.positions.get(id as usize))
        {
            Some(position) => self.orbit.focus(*position),
            None => self.orbit.show_everything(),
        }
    }

    fn load_scene(&mut self, context: &RenderContext, bytes: &[u8]) {
        match SceneData::parse(bytes) {
            Ok(scene) => self.scene = scene,
            Err(error) => {
                bridge::report(&format!("The scene could not be read: {error}"));
                return;
            }
        }
        let count = self.scene.positions.len();
        self.lit = vec![true; count];
        self.flash_at = vec![NEVER; count];
        self.filtering = false;
        self.selected = None;
        self.hovered = None;
        self.intro = if self.reduced_motion { 1.0 } else { 0.0 };
        self.frame(context);
        self.orbit.settle();
        self.instances_changed = true;

        let mut random = Random(0x5eed_1234);
        let shell = self.scene.radius.max(12.0);
        let stars: Vec<Instance> = (0..STAR_COUNT)
            .map(|_| {
                let direction = loop {
                    let point = Vec3::new(
                        random.between(-1.0, 1.0),
                        random.between(-1.0, 1.0),
                        random.between(-1.0, 1.0),
                    );
                    let length = point.length();
                    if length > 0.05 && length <= 1.0 {
                        break point / length;
                    }
                };
                let position = self.scene.centroid + direction * shell * random.between(3.0, 7.0);
                Instance {
                    a: [position.x, position.y, position.z, 0.0],
                    b: [STAR_COLOR[0], STAR_COLOR[1], STAR_COLOR[2], STATE_STAR],
                    c: [random.between(0.0, std::f32::consts::TAU), NEVER, 0.0, 0.0],
                }
            })
            .collect();
        if let Some(gpu) = &mut self.gpu {
            gpu.set_stars(context, &stars);
        }
        bridge::emit("notes:scene", count as f64);
    }

    fn set_matches(&mut self, matches: Option<Vec<u32>>) {
        self.filtering = matches.is_some();
        let flash_at = self.shader_seconds();
        match matches {
            None => self.lit.fill(true),
            Some(ids) => {
                self.lit.fill(false);
                for id in ids {
                    if let Some(lit) = self.lit.get_mut(id as usize) {
                        *lit = true;
                        self.flash_at[id as usize] = flash_at;
                    }
                }
            }
        }
        if self.selected.is_some_and(|id| !self.is_lit(id)) {
            self.select(None);
        }
        if self.hovered.is_some_and(|id| !self.is_lit(id)) {
            self.hover(None);
        }
        self.instances_changed = true;
    }

    fn is_lit(&self, id: u32) -> bool {
        self.lit.get(id as usize).copied().unwrap_or(false)
    }

    fn select(&mut self, id: Option<u32>) {
        let id = id.filter(|id| (*id as usize) < self.scene.positions.len());
        match id {
            Some(id) => self.orbit.focus(self.scene.positions[id as usize]),
            None => self.orbit.show_everything(),
        }
        if self.selected != id {
            self.selected = id;
            bridge::emit("notes:select", id.map_or(-1.0, f64::from));
        }
    }

    fn hover(&mut self, id: Option<u32>) {
        if self.hovered != id {
            self.hovered = id;
            bridge::emit("notes:hover", id.map_or(-1.0, f64::from));
        }
    }

    fn rebuild_instances(&mut self, context: &RenderContext) {
        let orbs: Vec<Instance> = self
            .scene
            .positions
            .iter()
            .enumerate()
            .map(|(index, position)| {
                let lit = self.lit[index];
                let color = self.scene.colors[index];
                Instance {
                    a: [
                        position.x,
                        position.y,
                        position.z,
                        if lit { ORB_SIZE } else { DIM_ORB_SIZE },
                    ],
                    b: [
                        color[0],
                        color[1],
                        color[2],
                        if lit { STATE_LIT } else { STATE_DIM },
                    ],
                    c: [
                        (index as f32 * 1.7) % std::f32::consts::TAU,
                        self.flash_at[index],
                        0.0,
                        0.0,
                    ],
                }
            })
            .collect();

        let mut random = Random(0x0011_cafe);
        let links: Vec<Instance> = self
            .scene
            .links
            .iter()
            .map(|link| {
                let from = self.scene.positions[link.from as usize];
                let to = self.scene.positions[link.to as usize];
                let lit = self.lit[link.from as usize] && self.lit[link.to as usize];
                Instance {
                    a: [from.x, from.y, from.z, random.next()],
                    b: [to.x, to.y, to.z, if link.explicit { 1.8 } else { 1.0 }],
                    c: [
                        if lit { 1.0 } else { 0.0 },
                        whole_turns(random.between(0.15, 0.4)),
                        0.0,
                        0.0,
                    ],
                }
            })
            .collect();

        if let Some(gpu) = &mut self.gpu {
            gpu.set_orbs(context, &orbs);
            gpu.set_links(context, &links);
        }
    }

    /// The lit note under a point given in physical pixels.
    fn pick(&self, context: &RenderContext, point: Vec2) -> Option<u32> {
        let view = self.view(context);
        let smallest = 12.0 * Self::ratio(context);
        let mut best: Option<(u32, f32)> = None;
        for (index, position) in self.scene.positions.iter().enumerate() {
            if !self.lit[index] {
                continue;
            }
            let Some(projected) = view.project(self.settled(*position)) else {
                continue;
            };
            let radius = view
                .pixel_radius(ORB_SIZE * 0.3, projected.depth)
                .max(smallest);
            let distance = projected.pixel.distance(point);
            if distance <= radius && best.is_none_or(|(_, nearest)| distance < nearest) {
                best = Some((index as u32, distance));
            }
        }
        best.map(|(id, _)| id)
    }

    /// The clock the shaders see, which wraps at `CLOCK_WRAP`.
    fn shader_seconds(&self) -> f32 {
        (self.clock % CLOCK_WRAP) as f32
    }

    fn eased_intro(&self) -> f32 {
        1.0 - (1.0 - self.intro).powi(3)
    }

    /// Where a note is during the intro, when the notes spread out from the
    /// centre.
    fn settled(&self, position: Vec3) -> Vec3 {
        self.scene.centroid + (position - self.scene.centroid) * self.eased_intro()
    }

    fn snapshot_labels(&self, context: &RenderContext) {
        let view = self.view(context);
        let ratio = Self::ratio(context);
        let size = Self::size(context);
        let limit = if self.filtering {
            LABELS_WHEN_FILTERING
        } else {
            LABELS
        };

        // The nearest notes, nearest first.
        let mut nearest: Vec<(u32, Vec2, f32)> = Vec::with_capacity(limit + 1);
        let mut pinned: Vec<(u32, Vec2, f32)> = Vec::with_capacity(2);
        // Labels wait until the notes have spread out.
        let spread_out = self.intro >= 1.0;
        for (index, position) in self.scene.positions.iter().enumerate() {
            if !spread_out || !self.lit[index] {
                continue;
            }
            let Some(projected) = view.project(self.settled(*position)) else {
                continue;
            };
            let pixel = projected.pixel;
            if pixel.x < 0.0 || pixel.y < 0.0 || pixel.x > size.x || pixel.y > size.y {
                continue;
            }
            let id = index as u32;
            let entry = (id, pixel, projected.depth);
            if self.selected == Some(id) || self.hovered == Some(id) {
                pinned.push(entry);
                continue;
            }
            if !self.filtering && projected.depth > self.orbit.distance * 1.05 {
                continue;
            }
            let at = nearest.partition_point(|other| other.2 <= projected.depth);
            if at < limit {
                nearest.insert(at, entry);
                nearest.truncate(limit);
            }
        }

        LABEL_SNAPSHOT.with(|snapshot| {
            let mut snapshot = snapshot.borrow_mut();
            snapshot.clear();
            for (id, pixel, depth) in pinned.into_iter().chain(nearest) {
                let mut flags = 0.0;
                if self.hovered == Some(id) {
                    flags += FLAG_HOVERED;
                }
                if self.selected == Some(id) {
                    flags += FLAG_SELECTED;
                }
                let radius = view.pixel_radius(ORB_SIZE * 0.25, depth).max(4.0 * ratio);
                snapshot.extend_from_slice(&[
                    id as f32,
                    pixel.x / ratio,
                    pixel.y / ratio,
                    radius / ratio,
                    flags,
                ]);
            }
        });
    }

    fn set_cursor(&mut self, context: &RenderContext, cursor: Cursor) {
        if self.cursor != cursor {
            self.cursor = cursor;
            bridge::set_cursor(
                context,
                match cursor {
                    Cursor::Grab => "grab",
                    Cursor::Grabbing => "grabbing",
                    Cursor::Pointer => "pointer",
                },
            );
        }
    }

    fn click(&mut self, context: &RenderContext, point: Vec2) {
        let picked = self.pick(context, point);
        self.select(picked);
    }

    fn pinch_span(&self) -> Option<f32> {
        match self.pointer.touches.as_slice() {
            [first, second] => Some(first.1.distance(second.1)),
            _ => None,
        }
    }
}

impl Example for Notes {
    fn settings(&self) -> ExampleSettings {
        ExampleSettings {
            title: "Pooya's Notes".to_owned(),
            // The scene is light, so the integrated GPU is enough and kinder
            // to a laptop's battery.
            power_preference: wgpu::PowerPreference::LowPower,
            ..ExampleSettings::default()
        }
    }

    fn init(&mut self, context: &mut RenderContext) -> RenderResult<()> {
        bridge::mount_canvas(context);
        self.srgb = context.surface_config.format.is_srgb();
        self.gpu = Some(Gpu::new(context));
        self.frame(context);
        self.orbit.settle();
        bridge::emit("notes:ready", 0.0);
        Ok(())
    }

    fn resize(&mut self, context: &mut RenderContext, _size: PhysicalSize<u32>) {
        self.frame(context);
    }

    fn input(&mut self, context: &mut RenderContext, event: &WindowEvent) -> bool {
        let ratio = Self::ratio(context);
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                let point = Vec2::new(position.x as f32, position.y as f32);
                if self.pointer.pressed {
                    if let Some(last) = self.pointer.cursor {
                        let moved = point - last;
                        self.pointer.travel += moved.length();
                        self.orbit.rotate(moved / ratio);
                    }
                } else {
                    let picked = self.pick(context, point);
                    self.hover(picked);
                    self.set_cursor(
                        context,
                        if picked.is_some() {
                            Cursor::Pointer
                        } else {
                            Cursor::Grab
                        },
                    );
                }
                self.pointer.cursor = Some(point);
            }
            WindowEvent::CursorLeft { .. } => {
                self.pointer.cursor = None;
                self.pointer.pressed = false;
                self.hover(None);
                self.set_cursor(context, Cursor::Grab);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => {
                    self.pointer.pressed = true;
                    self.pointer.travel = 0.0;
                    self.set_cursor(context, Cursor::Grabbing);
                }
                ElementState::Released => {
                    let was_pressed = std::mem::take(&mut self.pointer.pressed);
                    self.set_cursor(context, Cursor::Grab);
                    if let Some(point) = self.pointer.cursor
                        && was_pressed
                        && self.pointer.travel < 5.0 * ratio
                    {
                        self.click(context, point);
                    }
                }
            },
            WindowEvent::MouseWheel { delta, .. } => {
                let amount = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y * 0.12,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 / 400.0,
                };
                self.orbit.zoom((-amount).exp());
            }
            WindowEvent::Touch(touch) => {
                let point = Vec2::new(touch.location.x as f32, touch.location.y as f32);
                match touch.phase {
                    TouchPhase::Started => {
                        if self.pointer.touches.is_empty() {
                            self.pointer.travel = 0.0;
                        }
                        if self.pointer.touches.len() < 2 {
                            self.pointer.touches.push((touch.id, point));
                        }
                    }
                    TouchPhase::Moved => {
                        let before = self.pinch_span();
                        let mut moved = Vec2::ZERO;
                        for entry in &mut self.pointer.touches {
                            if entry.0 == touch.id {
                                moved = point - entry.1;
                                entry.1 = point;
                            }
                        }
                        self.pointer.travel += moved.length();
                        match (before, self.pinch_span()) {
                            (Some(before), Some(after)) if after > 1.0 => {
                                self.orbit.zoom(before / after);
                            }
                            _ => self.orbit.rotate(moved / ratio),
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        let alone = self.pointer.touches.len() == 1;
                        self.pointer.touches.retain(|entry| entry.0 != touch.id);
                        if alone
                            && touch.phase == TouchPhase::Ended
                            && self.pointer.travel < 10.0 * ratio
                        {
                            self.click(context, point);
                        }
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn update(&mut self, context: &mut RenderContext) {
        let commands = COMMANDS.with(|commands| std::mem::take(&mut *commands.borrow_mut()));
        for command in commands {
            match command {
                Command::LoadScene(bytes) => self.load_scene(context, &bytes),
                Command::Matches(matches) => self.set_matches(matches),
                Command::Select(id) => self.select(id),
                Command::ReducedMotion(reduced) => {
                    self.reduced_motion = reduced;
                    if reduced {
                        self.intro = 1.0;
                    }
                }
                Command::Paused(paused) => self.paused = paused,
                Command::Insets(insets) => {
                    self.insets = insets;
                    self.frame(context);
                }
            }
        }

        self.stats.tick();
        // A browser that is busy or in the background delivers few frames.
        // The intro follows the clock; the camera takes small steps.
        let elapsed = self.stats.delta_seconds().min(0.25);
        let seconds = elapsed.min(0.05);
        if self.paused {
            return;
        }
        let before = self.shader_seconds();
        self.clock += f64::from(elapsed);
        if self.shader_seconds() < before {
            // The shader clock has wrapped. A flash is timed on that clock, so
            // an old one would otherwise fire again when the clock next
            // passes its start.
            self.flash_at.fill(NEVER);
            self.instances_changed = true;
        }
        if !self.scene.positions.is_empty() {
            self.intro = (self.intro + elapsed / INTRO_SECONDS).min(1.0);
        }

        let resting = !self.pointer.pressed
            && self.pointer.touches.is_empty()
            && self.selected.is_none()
            && self.hovered.is_none();
        if resting && !self.reduced_motion {
            self.orbit.turn_idly(seconds);
        }
        self.orbit.advance(seconds, self.reduced_motion);

        if std::mem::take(&mut self.instances_changed) {
            self.rebuild_instances(context);
        }

        let view = self.view(context);
        let extent = self.scene.radius.max(10.0) * 5.0;
        let cell = 2.0_f32.powf((extent / 12.0).log2().round());
        let globals = Globals {
            view_projection: view.view_projection.to_cols_array_2d(),
            viewport: [view.size.x, view.size.y, view.offset.x, view.offset.y],
            time: [
                self.shader_seconds(),
                self.eased_intro(),
                if self.reduced_motion { 0.0 } else { 1.0 },
                if self.srgb { 1.0 } else { 0.0 },
            ],
            lens: [
                view.focal,
                Self::ratio(context),
                if self.filtering { 1.0 } else { 0.0 },
                self.orbit.fit(),
            ],
            floor: [
                self.scene.centroid.x,
                self.scene.floor,
                self.scene.centroid.z,
                extent,
            ],
            grid: [cell, 0.0, 0.0, 0.0],
            accent: [ACCENT[0], ACCENT[1], ACCENT[2], 1.0],
            centroid: [
                self.scene.centroid.x,
                self.scene.centroid.y,
                self.scene.centroid.z,
                0.0,
            ],
            detail: {
                let crowding = (SPARSE_NOTES / self.scene.positions.len().max(1) as f32)
                    .sqrt()
                    .min(1.0);
                [
                    LINK_RANGE,
                    crowding.max(0.12),
                    (5.0 * crowding).max(1.5),
                    0.0,
                ]
            },
        };
        if let Some(gpu) = &self.gpu {
            gpu.set_globals(context, &globals);
        }
        self.snapshot_labels(context);
    }

    fn render(
        &mut self,
        _context: &mut RenderContext,
        view: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) -> RenderResult<()> {
        let to_surface = |value: f64| if self.srgb { value.powf(2.2) } else { value };
        let clear = wgpu::Color {
            r: to_surface(CLEAR[0]),
            g: to_surface(CLEAR[1]),
            b: to_surface(CLEAR[2]),
            a: 1.0,
        };
        if let Some(gpu) = &self.gpu {
            gpu.draw(
                view,
                encoder,
                clear,
                self.paused || self.scene.positions.is_empty(),
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_pulse_is_back_at_its_start_when_the_clock_wraps() {
        for speed in [0.15_f32, 0.2371, 0.4] {
            let turns = whole_turns(speed) * CLOCK_WRAP as f32;
            assert!(
                (turns - turns.round()).abs() < 1.0e-3,
                "{speed} gives {turns} turns"
            );
            assert!((whole_turns(speed) - speed).abs() < 1.0 / CLOCK_WRAP as f32);
        }
    }

    #[test]
    fn the_shader_clock_wraps_but_keeps_its_precision() {
        let mut notes = Notes {
            clock: CLOCK_WRAP * 100.0 + 12.5,
            ..Notes::default()
        };
        assert!((notes.shader_seconds() - 12.5).abs() < 1.0e-3);

        // Ten days in, one frame at 120 Hz still moves the clock.
        notes.clock = 864_000.0;
        let before = notes.shader_seconds();
        notes.clock += 1.0 / 120.0;
        assert!(notes.shader_seconds() > before);
    }
}
