---
title: A web browser inside Unreal Engine 5.8, running in the browser
date: 2026-10-22
tags: [webgpu, unreal-engine, webrtc, cpp, wasm]
image: asset/ue5-web-browser-widget-webrtc-webgpu.jpg
image_alt: A YouTube page displayed on a world-space browser widget in Unreal Engine's First Person Shooter scene, running in Chrome through WebAssembly and WebGPU.
summary: Bringing Unreal's Web Browser widget to HTML5, then working through local & remote rendering.
---

After [bringing Unreal Engine to the browser](unreal-engine5-in-the-browser.md) and [connecting browser and native multiplayer clients](unreal-engine5-webgpu-and-networking.md), I wanted to put a website inside the game itself, Video textures were already working, including MP4, HLS and DASH. The next step was an interactive web page on a surface in the world, with other objects able to pass in front of it.

On native desktop builds, Unreal's Web Browser plugin has an embedded browser backend which is CEF for desktop and default web browser of mobile. Exporting the game to WebAssembly does not bring that backend with it. An iframe could display a website over the game canvas, but it would not become a texture with normal scene occlusion. Even controlling a website's CORS headers does not give JavaScript access to its rendered HTML pixels.

I settled on two paths. A cooperating application can supply an origin-clean canvas and handle input locally. That works for content designed to expose its rendering, rather than an arbitrary website. For ordinary websites, a separate Chromium process renders the page and sends its picture and audio over WebRTC. The game still runs locally through WebAssembly and WebGPU; only the web surface is streamed.

I wanted to keep Unreal's familiar workflow. The HTML5 extension therefore attaches to the existing Web Browser plugin and is included only when that plugin is enabled. Its original source files remain unchanged. C++ connects the widget to an Emscripten bridge written in TypeScript, and incoming frames are copied into WebGPU textures without passing CPU RGBA buffers through Wasm. A world-space Widget Component puts the result into the scene. Frame ownership and queues are bounded so a slow consumer cannot keep accumulating old pictures.

<iframe width="560" height="315" src="https://www.youtube.com/embed/1B80Y0Fo8FM?si=ig0N8POqXNN0tz5V" title="YouTube video player" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" referrerpolicy="strict-origin-when-cross-origin" allowfullscreen></iframe>

For the demo, I copied Epic's First Person template and used its Shooter variant. The screen opened a real YouTube page in an isolated Chromium session. Shooting became the mouse interaction: the weapon's aim trace checks the first blocking hit, and a hit on the browser surface becomes a pointer press and release through Unreal's Widget Interaction Component. Those events return to the remote page over a WebRTC data channel. A wall in front of the screen blocks the click too.

In short local tests at 1280 × 720 with the same 20 FPS cap, decoded video throughput rose from about 16.6 to 20 FPS, while median source-to-decoded-video age fell from 121 to 58 milliseconds. These measurements used a timestamped test page, not gameplay, and exclude the final Unreal presentation and monitor scanout. The full game had its own rendering cost, and occasional stalls remained. 

This is still an early implementation, validated locally on Chrome and macOS. The development renderer is a separate service, not something packaged into the game's Wasm. Production hosting needs authentication, session isolation, resource limits and network testing, and the widget still lacks parts of the native browser API. But the useful boundary is now clear: keep the game local, keep Unreal's widget workflow, and stream a web surface when the content cannot provide its own pixels.

The build guide and source-access instructions are in [UnrealEngine-WebGPU on GitHub](https://github.com/PooyaEimandar/UnrealEngine-WebGPU).
