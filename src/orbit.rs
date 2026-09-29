//! The orbiting camera, and the projection shared by drawing, picking and labels.

use sib::render::camera::Camera;
use sib::render::glam::{Mat4, Vec2, Vec3, Vec4};

pub const FOV_Y: f32 = std::f32::consts::FRAC_PI_4;
const MAX_PITCH: f32 = 1.35;
const IDLE_TURN_PER_SECOND: f32 = 0.11;

#[derive(Clone, Copy, Debug)]
pub struct Projected {
    /// Position in physical pixels, with the origin at the top left.
    pub pixel: Vec2,
    /// Distance along the view direction. Larger is farther away.
    pub depth: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct View {
    pub view_projection: Mat4,
    pub size: Vec2,
    /// Shift of the picture in normalised device coordinates, so the scene is
    /// centred in the part of the canvas that the interface does not cover.
    pub offset: Vec2,
    pub focal: f32,
}

impl View {
    pub fn project(&self, world: Vec3) -> Option<Projected> {
        let clip = self.view_projection * Vec4::new(world.x, world.y, world.z, 1.0);
        if clip.w <= 0.05 {
            return None;
        }
        let ndc = Vec2::new(clip.x, clip.y) / clip.w + self.offset;
        Some(Projected {
            pixel: Vec2::new(
                (ndc.x * 0.5 + 0.5) * self.size.x,
                (0.5 - ndc.y * 0.5) * self.size.y,
            ),
            depth: clip.w,
        })
    }

    /// Radius in physical pixels of something `radius` wide at `depth`.
    pub fn pixel_radius(&self, radius: f32, depth: f32) -> f32 {
        radius * self.focal / depth * self.size.y * 0.5
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub target: Vec3,
    pub offset: Vec2,
    goal_distance: f32,
    goal_target: Vec3,
    goal_offset: Vec2,
    home: Vec3,
    fit: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        Self {
            yaw: 0.6,
            pitch: 0.3,
            distance: 30.0,
            target: Vec3::ZERO,
            offset: Vec2::ZERO,
            goal_distance: 30.0,
            goal_target: Vec3::ZERO,
            goal_offset: Vec2::ZERO,
            home: Vec3::ZERO,
            fit: 30.0,
        }
    }
}

impl Orbit {
    /// Sets the distance from which the whole scene is visible.
    pub fn frame(&mut self, centre: Vec3, radius: f32, aspect: f32) {
        let narrow = aspect.clamp(0.4, 1.0);
        self.fit = radius.max(5.0) * 1.25 / (FOV_Y * 0.5).tan() / narrow;
        self.home = centre;
    }

    pub fn fit(&self) -> f32 {
        self.fit
    }

    pub fn show_everything(&mut self) {
        self.goal_target = self.home;
        self.goal_distance = self.fit;
    }

    pub fn focus(&mut self, position: Vec3) {
        self.goal_target = position;
        self.goal_distance = (self.fit * 0.45).max(7.0);
    }

    pub fn set_offset(&mut self, offset: Vec2) {
        self.goal_offset = offset;
    }

    pub fn rotate(&mut self, pixels: Vec2) {
        self.yaw -= pixels.x * 0.006;
        self.pitch = (self.pitch + pixels.y * 0.006).clamp(-MAX_PITCH, MAX_PITCH);
    }

    pub fn zoom(&mut self, factor: f32) {
        self.goal_distance = (self.goal_distance * factor).clamp(3.0, self.fit * 2.5);
    }

    pub fn turn_idly(&mut self, seconds: f32) {
        self.yaw += seconds * IDLE_TURN_PER_SECOND;
    }

    /// Moves towards the goals. With `instant` the camera jumps, which is what
    /// people who asked for reduced motion get.
    pub fn advance(&mut self, seconds: f32, instant: bool) {
        let blend = if instant {
            1.0
        } else {
            1.0 - (-seconds * 5.0).exp()
        };
        self.target += (self.goal_target - self.target) * blend;
        self.distance += (self.goal_distance - self.distance) * blend;
        self.offset += (self.goal_offset - self.offset) * blend;
    }

    /// Jumps straight to the goals.
    pub fn settle(&mut self) {
        self.advance(0.0, true);
    }

    pub fn eye(&self) -> Vec3 {
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        self.target + Vec3::new(cos_pitch * sin_yaw, sin_pitch, cos_pitch * cos_yaw) * self.distance
    }

    pub fn view(&self, size: Vec2, reach: f32) -> View {
        let mut camera = Camera::new(self.eye(), self.target, size.x / size.y.max(1.0));
        camera.fovy_radians = FOV_Y;
        camera.znear = 0.1;
        camera.zfar = reach.max(64.0);
        View {
            view_projection: camera.view_projection_matrix(),
            size,
            offset: self.offset,
            focal: 1.0 / (FOV_Y * 0.5).tan(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn the_target_is_in_the_middle_of_the_picture() {
        let mut orbit = Orbit::default();
        orbit.frame(Vec3::new(1.0, 2.0, 3.0), 10.0, 1.6);
        orbit.show_everything();
        orbit.settle();
        let view = orbit.view(Vec2::new(1600.0, 1000.0), 500.0);
        let projected = view.project(orbit.target).expect("the target is in front of the camera");
        assert!(close(projected.pixel.x, 800.0));
        assert!(close(projected.pixel.y, 500.0));
        assert!(close(projected.depth, orbit.distance));
    }

    #[test]
    fn the_offset_moves_the_picture() {
        let mut orbit = Orbit::default();
        orbit.set_offset(Vec2::new(-0.5, 0.25));
        orbit.settle();
        let view = orbit.view(Vec2::new(1000.0, 800.0), 500.0);
        let projected = view.project(orbit.target).expect("the target is in front of the camera");
        assert!(close(projected.pixel.x, 250.0));
        assert!(close(projected.pixel.y, 300.0));
    }

    #[test]
    fn nothing_behind_the_camera_is_projected() {
        let orbit = Orbit::default();
        let view = orbit.view(Vec2::new(1000.0, 800.0), 500.0);
        let behind = orbit.eye() + (orbit.eye() - orbit.target);
        assert!(view.project(behind).is_none());
    }

    #[test]
    fn a_narrow_screen_steps_back() {
        let mut wide = Orbit::default();
        wide.frame(Vec3::ZERO, 10.0, 1.6);
        let mut narrow = Orbit::default();
        narrow.frame(Vec3::ZERO, 10.0, 0.5);
        assert!(narrow.fit() > wide.fit());
    }

    #[test]
    fn focusing_moves_closer() {
        let mut orbit = Orbit::default();
        orbit.frame(Vec3::ZERO, 20.0, 1.6);
        orbit.show_everything();
        orbit.settle();
        let far = orbit.distance;
        orbit.focus(Vec3::new(4.0, 0.0, 0.0));
        orbit.settle();
        assert!(orbit.distance < far);
        assert_eq!(orbit.target, Vec3::new(4.0, 0.0, 0.0));
    }
}
