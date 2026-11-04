---
title: When Emscripten exits a thread that is still running
date: 2026-11-04
tags: [unreal-engine, wasm, cpp, webgpu]
summary: My Unreal Engine browser build ran in Chrome but stopped in Safari with a ToBigInt error. The game thread was being exited while JSPI had suspended its main().
---

My [Unreal Engine 5.8 browser build](unreal-engine5-in-the-browser.md) ran in Chrome, but in Safari 27, and in a macOS app that embeds the same WebKit, it never got past the loading screen. The engine's workers kept reporting one line: `TypeError: Invalid argument type in ToBigInt operation`.

Some context first. The build runs `main()` on a Web Worker (`-sPROXY_TO_PTHREAD`), so the engine loop can block as it does on other platforms. When it waits for the GPU, JSPI suspends the WebAssembly stack and hands control back to the worker's event loop. Pointers are 64-bit at the JavaScript boundary (`-sMEMORY64=2`), so the generated glue passes them as `BigInt`.

A worker's error event carries only a message, a file and a line, and the line pointed at Emscripten's thread start. To get a real stack, I wrapped `Worker` before the engine's script ran, so each engine worker registered its own error and rejection handlers first and posted the stack back to the page. Safari's stack ended in a plain `BigInt(...)` call, reached from Emscripten's callback wrapper after a WebGPU `onSubmittedWorkDone` promise had resolved.

Emscripten runs such callbacks through `callUserCallback`, which finishes with `maybeExit`. When nothing holds a runtime keepalive, `maybeExit` treats the pthread as done and calls `_emscripten_thread_exit(EXITSTATUS)`. But this thread was not done: its `main()` was suspended in a GPU wait, and that wait held no keepalive. `EXITSTATUS` was still `undefined`, and the 64-bit wrapper's `BigInt(undefined)` threw. Chrome happened to avoid this path, and I have not traced why. With 32-bit pointers, I would expect the same path to exit the game thread silently instead of throwing.

I confirmed it twice before touching the engine. Patching the loader in Safari to hold a keepalive around the thread's entry point let the scene render, with the first frame at 12.2 s. Then a minimal Emscripten 6.0.10 program with the engine's flags, where a timer callback completes while `main()` waits in an async library function that holds no keepalive, failed the same way under Node, in V8's wording: `Cannot convert undefined to a BigInt`. With a keepalive, it resumed and returned normally.

The fix is small. In the HTML5 launch code, `main()` calls `emscripten_runtime_keepalive_push()` on entry and releases it with `ON_SCOPE_EXIT`, so every return path, early failures included, still exits the thread and reports its exit code to the page.

The lesson I took from it: with JSPI, a suspended `main()` is still a running thread, but Emscripten's exit check only counts keepalives. Anything that suspends without one is at the mercy of whichever callback finishes next, and a single browser's timing can hide that for a long time.

The build guide and source-access instructions are in [UnrealEngine-WebGPU on GitHub](https://github.com/PooyaEimandar/UnrealEngine-WebGPU).
