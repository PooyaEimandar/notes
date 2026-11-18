---
title: Water simulation in the browser with WebGPU
date: 2026-11-15
tags: [webgpu, rust, graphics, wasm]
image: https://pooya.ai/webgpu/screenshots/computefluid.webp
image_alt: Foam and splashes around a copper-colored floating ball in a tiled water basin.
summary: Building a water simulation with Rust and WebGPU, with a floating ball, ripples, foam, and splashes.
---

For this example in [my WebGPU examples](my-webgpu-examples.md), I built a small pool with water flowing from a pipe and a ball floating on the surface. You can drag the ball through the water, leave a wake behind it, and watch the pool settle after you let go. The example uses Rust, wgpu, and WGSL through [sib::render](https://github.com/PooyaEimandar/sib), and runs both as a native app and in the browser with WebGPU.

The water starts with 2,048 particles. Half begin in the pool, and the rest enter through the pipe in small groups. Each particle stores its position, speed, and other water data on the GPU. Compute shaders update this data, and the renderer reads it directly to draw the next frame. The physics moves forward in steps of 1/60 of a second. Only a small amount of ball data returns to the CPU from time to time, so the controls can follow its movement.

The main problem is keeping the particles close enough to look like water without letting them pile up in the same place. I used [Position Based Fluids, a method by Miles Macklin and Matthias Müller](https://www.researchgate.net/publication/260398844_Position_based_fluids). It checks how tightly nearby particles are packed and makes small changes to their positions when they are too crowded. A 3D grid helps each particle find its neighbors without searching through the whole pool. The solver repeats these checks twice by default, and you can increase that to six rounds from the controls.

The ball and water affect each other. The GPU checks nearby particles to estimate the water level and flow around the ball. Gravity pulls it down, while the water pushes it up based on how much of the ball is underwater. Drag helps it move with the flow, and the water receives an opposite push. Moving the ball also pushes particles out of its way.

Drawing the particles as separate dots would leave a rough surface with gaps. To make them look like water, I draw overlapping shapes and record their distance from the camera. Several filters smooth this depth image while keeping large depth changes apart. Another pass estimates how thick the water is. These results control the reflections, the bent view through the water, and its color. The basin, pipe, colors, and lighting are all created in code. Particle view lets you see the points underneath the smooth surface, colored by speed.

Small ripples need more detail than this particle count can show. I added a separate grid that stores wave height and motion across the pool. A moving ball creates waves in front of it and a wake behind it. The grid remembers those waves, so they keep spreading and fading after the ball turns or stops. They change the visible surface and its reflections, but they do not push the particles or ball. Foam and spray are also visual effects. They appear around the moving ball, follow the water or fly through the air, then fade or fall back into the pool. To reduce the drawing cost, the water, foam, and spray use a smaller image that is enlarged to fill the display.

You can [try the fluid simulation in your browser](https://pooya.ai/webgpu/computefluid/) and [read the source on GitHub](https://github.com/PooyaEimandar/webgpu/blob/main/examples/computefluid.rs).
