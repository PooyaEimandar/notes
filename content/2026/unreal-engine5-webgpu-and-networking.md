---
title: Unreal Engine 5, WebGPU and multiplayer across native and browser
date: 2026-10-19
tags: [webgpu, unreal-engine, rust, cpp, wasm]
image: asset/unreal-engine-webgpu-networking.jpg
image_alt: Several Unreal Engine Third Person clients sharing a multiplayer scene, with Unreal Engine, heart, WebGPU and networking graphics.
summary: Connecting browser players to Unreal's native multiplayer server through a Rust gateway, and what reconnects, the Third Person template and memory profiling taught me along the way.
---

After [bringing Unreal Engine to the browser](unreal-engine5-in-the-browser.md), I had two approaches to consider for multiplayer. I could implement WebSocket and WebTransport server support inside Unreal's C++ environment, through a plugin or changes to the fork. Alternatively, I could build a separate gateway that maps browser WebSocket and WebTransport connections onto Unreal's existing UDP networking. Browser JavaScript and WebAssembly cannot open raw UDP sockets, so either approach needed to bridge that gap.

There were already useful pieces inside the engine. Unreal includes an experimental WebSocketNetworking plugin with a server implementation and its own net driver, and it includes [MsQuic](https://github.com/microsoft/msquic) for features such as [QUIC Messaging](https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Plugins/QuicMessaging). In the Unreal 5.8 code I am working with, that messaging transport serves Unreal's Message Bus; the default gameplay driver, `IpNetDriver`, still uses UDP.

Building on those components was possible, but I wanted to limit the engine integration I would need to maintain as Unreal evolved. A plugin would still need maintenance across releases, and deeper changes to the fork could make future upgrades harder. I chose the gateway approach and built it on [Sib](https://github.com/PooyaEimandar/sib), my open-source high-performance Rust modules for rendering, networking, streaming, and scalable services. It would replace the previous Python host, taking responsibility for both serving the packaged assets and handling browser multiplayer connections.

I added a socket adapter to the HTML5 platform. It implements the client datagram operations Unreal needs through its `FSocket` interface. C++ calls an Emscripten bridge generated from TypeScript, which uses the browser's WebTransport or WebSocket API. Browser events arrive asynchronously, so bounded queues sit between those callbacks and the engine's networking work. Unreal's `IpNetDriver`, replication and RPC code remain unchanged.

The gateway gives each browser session its own UDP endpoint connected to the configured Unreal server. It forwards complete Unreal packets, while the dedicated server owns the shared game state. Native clients continue to connect directly over UDP. That separation lets both kinds of client participate in the same game using Unreal's existing networking behavior.

The same Rust host serves the WebAssembly, JavaScript and game assets, including HTTP byte ranges for streamed content. It supports HTTPS with HTTP/2 and HTTP/1.1 fallback. Hosting and gateway networking share one Tokio runtime in their own process, with blocking file work kept off the networking tasks. Packaging now bundles that executable, so running an exported game needs neither Python nor a Rust installation.

Choosing two browser transports meant accepting two different behaviors. WebTransport datagrams support unreliable delivery, which fits Unreal's packet-based networking. WebSocket delivers messages reliably and in order, so packet loss can hold up later gameplay messages. I kept secure WebSocket as a fallback during connection establishment and tested both paths separately. Changing transport during an established game still requires reconnecting.

<iframe width="560" height="315" src="https://www.youtube.com/embed/6WJdi4Iynrc?si=p_RnNxuOrA11jNj-" title="YouTube video player" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" referrerpolicy="strict-origin-when-cross-origin" allowfullscreen></iframe>

For the visible demo, I copied Epic's official Third Person template and kept its map, character, animations and input mappings. A packaged macOS client rendered through Metal, while Chrome clients ran the game locally through WebAssembly and WebGPU. Opening more tabs brought more players into the shared world.

There is more work before I can call this production-ready. Synthetic gateway load tests exposed loss and long delays under pressure; they cannot establish how many players a real Unreal world can support. My preferred next step is to host a multiplayer game on a website and test it with real players, across different devices and network conditions.

