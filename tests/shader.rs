//! The shader is compiled by the browser at run time, so check it here first.

use naga::valid::{Capabilities, ValidationFlags, Validator};

#[test]
fn the_scene_shader_is_valid() {
    let source = include_str!("../src/scene.wgsl");
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
    Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));

    let entry_points: Vec<&str> = module
        .entry_points
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    for expected in ["vs_orb", "fs_orb", "vs_link", "fs_link", "vs_floor", "fs_floor"] {
        assert!(entry_points.contains(&expected), "{expected} is missing");
    }
}
