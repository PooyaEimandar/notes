// Everything is drawn with additive blending and no depth buffer, so the draw
// order does not matter.

struct Globals {
    view_projection: mat4x4<f32>,
    // width, height, picture offset x, picture offset y
    viewport: vec4<f32>,
    // seconds, intro (0..1), motion (0 or 1), surface is sRGB (0 or 1)
    time: vec4<f32>,
    // focal length, pixel ratio, filtering (0 or 1), overview distance
    lens: vec4<f32>,
    // centre x, floor y, centre z, extent
    floor: vec4<f32>,
    // grid cell, 0, 0, 0
    grid: vec4<f32>,
    accent: vec4<f32>,
    centroid: vec4<f32>,
    // how far links are drawn, orb exposure, smallest orb in pixels, 0
    detail: vec4<f32>,
}

@group(0) @binding(0) var<uniform> g: Globals;

const STATE_DIM: f32 = 0.0;
const STATE_LIT: f32 = 1.0;
const STATE_STAR: f32 = 2.0;

fn output(rgb: vec3<f32>) -> vec4<f32> {
    if (g.time.w > 0.5) {
        return vec4<f32>(pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(2.2)), 1.0);
    }
    return vec4<f32>(rgb, 1.0);
}

fn settle(world: vec3<f32>) -> vec3<f32> {
    return g.centroid.xyz + (world - g.centroid.xyz) * g.time.y;
}

// ---------------------------------------------------------------- orbs, stars

struct OrbOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
    // state, boost, brightness
    @location(2) params: vec3<f32>,
}

@vertex
fn vs_orb(
    @builtin(vertex_index) index: u32,
    // position, size
    @location(0) a: vec4<f32>,
    // colour, state
    @location(1) b: vec4<f32>,
    // phase, flash start, 0, 0
    @location(2) c: vec4<f32>,
) -> OrbOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[index];
    let state = b.w;
    let seconds = g.time.x;
    let motion = g.time.z;

    var world = a.xyz;
    var size = a.w;
    var boost = 0.0;
    var brightness = 1.0;
    var smallest = g.detail.z;

    if (state > 1.5) {
        brightness = 0.3 + 0.25 * sin(seconds * 1.5 * motion + c.x);
        smallest = 1.2;
    } else {
        world = settle(a.xyz);
        if (state > 0.5) {
            size = size * (1.0 + 0.14 * sin(seconds * 2.2 + c.x) * motion);
            let since = seconds - c.y;
            if (since >= 0.0) {
                boost = exp(-since * 3.0) * motion;
            }
            size = size * (1.0 + boost);
        }
    }

    let clip = g.view_projection * vec4<f32>(world, 1.0);
    let depth = max(clip.w, 0.0001);
    if (state < 1.5) {
        // Far away, many orbs share a pixel. Turn them down so they do not
        // add up to white.
        let near = 1.0 - smoothstep(g.detail.x * 0.5, g.detail.x * 1.5, depth);
        let exposure = mix(g.detail.y, max(g.detail.y, 0.45), near);
        brightness = clamp(1.45 - depth / (g.lens.w * 1.6), 0.4, 1.0) * exposure * g.time.y;
    }

    let half_y = max(size * g.lens.x / depth, smallest * g.lens.y * 2.0 / g.viewport.y);
    let half = vec2<f32>(half_y * g.viewport.y / g.viewport.x, half_y);

    var out: OrbOut;
    out.position = vec4<f32>(clip.xy + (corner * half + g.viewport.zw) * clip.w, clip.z, clip.w);
    out.uv = corner;
    out.color = b.xyz;
    out.params = vec3<f32>(state, boost, brightness);
    return out;
}

@fragment
fn fs_orb(in: OrbOut) -> @location(0) vec4<f32> {
    let distance = length(in.uv);
    if (distance >= 1.0) {
        discard;
    }
    let state = in.params.x;
    var rgb: vec3<f32>;
    if (state > 1.5) {
        rgb = in.color * (1.0 - smoothstep(0.0, 1.0, distance));
    } else if (state > 0.5) {
        let edge = 1.0 - smoothstep(0.7, 1.0, distance);
        let core = 1.0 - smoothstep(0.04, 0.13, distance);
        let glow = exp(-distance * distance * 5.0);
        let halo = exp(-distance * 2.6) * 0.35;
        rgb = (in.color * (glow + halo) * (0.9 + in.params.y) + vec3<f32>(core)) * edge;
    } else {
        rgb = vec3<f32>(0.07, 0.15, 0.17) * (1.0 - smoothstep(0.1, 0.35, distance));
    }
    return output(rgb * in.params.z);
}

// ---------------------------------------------------------------------- links

struct LinkOut {
    @builtin(position) position: vec4<f32>,
    // along the link (0..1), across it in pixels
    @location(0) uv: vec2<f32>,
    // length in pixels, phase, speed, lit
    @location(1) params: vec4<f32>,
    @location(2) strength: f32,
}

@vertex
fn vs_link(
    @builtin(vertex_index) index: u32,
    // start, phase
    @location(0) a: vec4<f32>,
    // end, strength
    @location(1) b: vec4<f32>,
    // lit, speed, 0, 0
    @location(2) c: vec4<f32>,
) -> LinkOut {
    var ends = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    var sides = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let end = ends[index];
    let side = sides[index];

    var start = g.view_projection * vec4<f32>(settle(a.xyz), 1.0);
    var finish = g.view_projection * vec4<f32>(settle(b.xyz), 1.0);

    var out: LinkOut;
    out.uv = vec2<f32>(0.0);
    out.params = vec4<f32>(0.0);
    out.strength = 0.0;

    // Cut the link where it passes behind the camera.
    let near = 0.05;
    if (start.w < near && finish.w < near) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    // Links are drawn only near the camera. From far away there are too many
    // of them to read, and they are costly to draw.
    let visible = 1.0 - smoothstep(g.detail.x * 0.6, g.detail.x, (start.w + finish.w) * 0.5);
    if (visible <= 0.001) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    if (start.w < near) {
        start = mix(start, finish, (near - start.w) / (finish.w - start.w));
    } else if (finish.w < near) {
        finish = mix(finish, start, (near - finish.w) / (start.w - finish.w));
    }

    let half_view = g.viewport.xy * 0.5;
    let start_ndc = start.xy / start.w + g.viewport.zw;
    let finish_ndc = finish.xy / finish.w + g.viewport.zw;
    let span = (finish_ndc - start_ndc) * half_view;
    let pixels = length(span);
    var along = vec2<f32>(1.0, 0.0);
    if (pixels > 0.0001) {
        along = span / pixels;
    }
    let across = vec2<f32>(-along.y, along.x);
    let half_width = 3.5 * g.lens.y;

    let centre = mix(start, finish, end);
    let ndc = mix(start_ndc, finish_ndc, end) + across * side * half_width / half_view;
    out.position = vec4<f32>(ndc * centre.w, centre.z, centre.w);
    out.uv = vec2<f32>(end, side * half_width);
    out.params = vec4<f32>(pixels, a.w, c.y, c.x);
    out.strength = b.w * visible;
    return out;
}

@fragment
fn fs_link(in: LinkOut) -> @location(0) vec4<f32> {
    let ratio = g.lens.y;
    let lit = in.params.w;
    let line = 1.0 - smoothstep(0.4 * ratio, 1.1 * ratio, abs(in.uv.y));
    let level = mix(0.04, mix(0.2, 0.4, g.lens.z), lit) * in.strength;
    var rgb = g.accent.rgb * line * level;

    if (lit > 0.5 && g.time.z > 0.5) {
        let head = fract(g.time.x * in.params.z + in.params.y);
        let away = vec2<f32>((in.uv.x - head) * in.params.x, in.uv.y);
        let pulse = 1.0 - smoothstep(1.0 * ratio, 2.6 * ratio, length(away));
        rgb = rgb + vec3<f32>(0.76, 0.96, 1.0) * pulse * 0.9 * min(in.strength, 1.0);
    }
    return output(rgb * g.time.y);
}

// ---------------------------------------------------------------------- floor

struct FloorOut {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec2<f32>,
}

@vertex
fn vs_floor(@builtin(vertex_index) index: u32) -> FloorOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let flat = corners[index] * g.floor.w;
    let world = vec3<f32>(g.floor.x + flat.x, g.floor.y, g.floor.z + flat.y);
    let clip = g.view_projection * vec4<f32>(world, 1.0);

    var out: FloorOut;
    out.position = vec4<f32>(clip.xy + g.viewport.zw * clip.w, clip.z, clip.w);
    out.world = flat;
    return out;
}

@fragment
fn fs_floor(in: FloorOut) -> @location(0) vec4<f32> {
    let extent = g.floor.w;
    let reach = length(in.world);
    let fade = 1.0 - smoothstep(extent * 0.3, extent * 0.95, reach);

    let cell = in.world / g.grid.x;
    let width = fwidth(cell);
    let gap = abs(fract(cell - 0.5) - 0.5) / max(width, vec2<f32>(0.00001));
    let line = 1.0 - min(min(gap.x, gap.y), 1.0);
    // Far away the lines crowd together; fade them before they shimmer.
    let calm = clamp(0.12 / max(width.x, width.y), 0.0, 1.0);

    // The radar ring travels outwards from the centre and fades as it goes.
    let phase = fract(g.time.x * 0.25);
    let ring_at = phase * extent * 0.6;
    let ring_width = max(fwidth(reach) * 1.5, 0.0001);
    let ring = (1.0 - smoothstep(0.0, ring_width, abs(reach - ring_at))) * (1.0 - phase) * g.time.z;

    let rgb = g.accent.rgb * (line * calm * 0.16 + ring * 0.4) * fade * g.time.y;
    return output(rgb);
}
