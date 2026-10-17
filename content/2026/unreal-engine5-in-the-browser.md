---
title: Bringing Unreal Engine 5.8 to the browser
date: 2026-10-17
tags: [webgpu, unreal-engine, cpp, wasm]
image: asset/unreal-engine-webgpu-thumbnail.jpg
image_alt: The Unreal Engine logo, a red heart, and the WebGPU logo on a dark blue background.
summary: From a native WebGPU renderer to packaging an Unreal Engine game for the browser, a few things I learned while building UnrealEngine-WebGPU.
---

After building my [WebGPU examples](my-webgpu-examples.md), I wanted to take the next step: bring a complete game engine into the browser. That became UnrealEngine-WebGPU, my fork of Unreal Engine 5.8.3 with a WebGPU renderer and an HTML5 platform target.

Unreal's mobile renderer already ran well through Metal at the engine's `ES3_1` feature level. That made me almost certain, at the start, that porting this rendering path to WebGPU would be straightforward.

I started with native rendering. Unreal's Rendering Hardware Interface, or RHI, is the layer between the engine and the graphics API. Since Unreal's engine code is C++, I chose Dawn to keep the rendering integration consistent with the existing C++ codebase.

[Dawn](https://dawn.googlesource.com/dawn/+/refs/heads/main/README.md) is an open-source implementation of WebGPU, also used by Chromium. It provides C/C++ interfaces and implements WebGPU on top of native graphics APIs such as Metal, Vulkan and Direct3D 12. I built my WebGPU RHI around it and got a scene running on my Mac through Metal. Working natively first gave me a place to debug rendering before adding WebAssembly and the browser's execution model.

Then came the browser. Emscripten compiles the engine's C++ into WebAssembly, while the same RHI calls the browser's WebGPU API through the emdawnwebgpu bindings. Shaders are translated during cooking: DXC compiles Unreal's HLSL to SPIR-V, then Tint, Dawn's shader compiler, converts it to WGSL. The current path uses Unreal's mobile renderer; Nanite and Lumen remain outside its scope.

I would love to bring my [Nanite-inspired work in Rust and WebGPU](https://pooya.ai/webgpu/nanite/) into this project. This would be a separate implementation adapted to WebGPU, not Unreal Engine's Nanite, and I would not expect it to match Nanite's features or performance.

Getting a scene to draw was only part of the work. The browser has its own rules around threads, asynchronous GPU operations and file access. This browser build requires cross-origin isolation for shared memory and uses WebAssembly JSPI to suspend execution during asynchronous GPU waits. Engine content arrives over HTTP. I added browser services for input, audio and persistent saves, with asset data fetched through HTTP range requests as the engine needs it.

Some of the most useful lessons came from ordinary packaging failures. A desktop project could enable rendering features that the browser path did not support. For example, my test project's Virtual Textures setting overrode the HTML5 defaults, causing cooking to fail on virtual-texture feedback shaders that Tint could not translate. I fixed this by moving the browser's rendering limits from `BaseHTML5Engine.ini` to `HTML5Engine.ini`, which loads after the project's defaults. The order of Unreal's configuration layers mattered as much as the settings themselves. Even SDK discovery needed attention: an editor launched from Finder did not inherit the same environment as my terminal.

The workflow now reaches the editor: select HTML5, package the project, and run the included local server. Seeing the familiar first-person scene respond inside a browser tab made all those pieces feel connected.

Development and build validation are currently on macOS, with Windows and Linux planned. I am focusing on making the core platform reliable before expanding into multiplayer and multimedia. For both, I want a clean integration that keeps further changes to Unreal's core code to a minimum, so I need more time to design the architecture. There is still work ahead, but there is now a game to play and a workflow others can try.

I hope Epic eventually brings back official support for running Unreal games in the browser. The move away from official HTML5 support was announced with [Unreal Engine 4.23](https://www.unrealengine.com/blog/unreal-engine-4-23-released?lang=en-US), and HTML5 became a [community-supported platform extension in 4.24](https://github.com/UnrealEngineHTML5/Documentation/blob/master/Platforms/HTML5/HowTo/README.1.emscripten.UE4.HTML5.md). I can understand the decision, given the constraints of the WebGL-based browser path at the time. Still, I hope Epic gives the web another chance with WebGPU, and that work like this eventually becomes part of an official release maintained by Unreal's development team.

You can watch the [browser demo](https://youtu.be/AyVYaKl-kJY?si=t6LkXTmN5Y5jSwSM) on YouTube. The build guide and source-access instructions are in [UnrealEngine-WebGPU on GitHub](https://github.com/PooyaEimandar/UnrealEngine-WebGPU).

<div class="video">
<iframe width="560" height="315" src="https://www.youtube.com/embed/AyVYaKl-kJY" title="Unreal Engine 5.8.3 in Your Browser! | WebGPU + WebAssembly Demo" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" referrerpolicy="strict-origin-when-cross-origin" allowfullscreen loading="lazy"></iframe>
</div>
