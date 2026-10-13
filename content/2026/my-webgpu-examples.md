---
title: My WebGPU examples
date: 2026-10-13
tags: [webgpu, graphics, rust]
image: https://pooya.ai/webgpu/screenshots/residentevil2.jpg
image_alt: Claire Redfield standing in the rain outside the R.P.D. gate, rendered in WebGPU with fixed cameras.
summary: WebGPU has arrived in the browser. Here is why I think it matters, and the examples I built by porting Sascha Willems' Vulkan samples to Rust and WGPU.
---

While the newest generation of rendering is moving toward neural rendering, the web has finally got a modern GPU API of its own. WebGPU replaces WebGL, it ships today in Chrome, Safari, and Firefox, and for the first time it brings compute shaders to the web as well.

Its pipeline reminds me of the migration from DirectX 9 to DirectX 10 and 11, or, in OpenGL terms, from OpenGL 3.1 to OpenGL 3.2 and 4.0. The industry handed developers some very cool features: the geometry stage came with DirectX 10 and OpenGL 3.2, and the tessellation stages with DirectX 11 and OpenGL 4.0.

After many years, most developers reached the same conclusion: almost everything the geometry and tessellation shaders do, at a high cost, a compute shader can do at a lower one. The native APIs eventually replaced those stages with mesh shaders.

WebGPU had one more constraint on top: it has to run everywhere, including Metal, which never had geometry shaders, and mobile GPUs, where tessellation is weak or missing. So, as far as the shader stages go, WebGPU is WebGL with the vertex and fragment stages, plus compute. The rest of it is new: explicit pipelines, bind groups, and command buffers in place of WebGL's global state, storage buffers, and the WGSL shading language, and that is where most of its speed comes from.

I believe that with WebGPU, what a browser can do in gaming and rendering takes a huge step forward. Techniques that WebGL could never run, such as GPU-driven culling, particle and cloth simulation, and ray tracing in compute shaders, now work in a tab, and games that until now needed a native build can reach anyone with a link.

A while ago, I decided to build a set of [WebGPU examples](https://pooya.ai/webgpu/) that teach it step by step, and I started by porting [Sascha Willems' Vulkan examples](https://github.com/SaschaWillems/vulkan). I ported most of his awesome code, not to C++ and Dawn, but to Rust and the WGPU ecosystem. After that, I extended the examples with some ideas of my own, such as a scene with the fixed cameras of [Resident Evil 2](https://pooya.ai/webgpu/residentevil2/), and [a game](https://pooya.ai/webgpu/geometrydash/) I made together with my eight-year-old son, and published them in the same place. I explained each sample with the help of AI, so anyone who wants to start learning WebGPU can do it step by step.

The examples run in the browser at [pooya.ai/webgpu](https://pooya.ai/webgpu/), and their source is on [GitHub](https://github.com/PooyaEimandar/webgpu). As for neural rendering, it is the part I find most interesting of all. I am working on it, and I will share what I learn here later.

By the way, if you are looking for Vulkan tutorials, check out my [Vulkan from Zero to Hero](https://www.youtube.com/watch?v=rdapoaS-oIM) series, which you can watch as a playlist on my [YouTube channel](https://www.youtube.com/channel/UC5XZoDB5YHd07WSWeMAYyZQ).

<div class="video">
<iframe width="560" height="315" src="https://www.youtube.com/embed/rdapoaS-oIM?si=jQ2K92TI3gqEYF1T" title="Vulkan from Zero to Hero: Welcome" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" referrerpolicy="strict-origin-when-cross-origin" allowfullscreen loading="lazy"></iframe>
</div>
