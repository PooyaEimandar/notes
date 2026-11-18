---
title: Ainu Capital Dev Journey
date: 2026-11-18
tags: [rust, wasm, crypto, security]
image: asset/ainu-capital-dev-journey.jpg
image_alt: The AINU Capital landing page, which displays market forecasts and prediction accuracy.
summary: AINU Capital keeps every wallet decision in one Rust core that runs in the browser and in native apps, behind serverless workers, which are based on Rust too. These are the problems I ran into and how I solved them.
---

[AINU Capital](https://capital.ainu.ae) is a crypto market product with a self-custody wallet. Live prices and forecasts on one side, and a wallet whose keys only its owner holds on the other. AINU keeps no private key and no encrypted copy of one.

The wallet, backend, forecast feed and native app commands are written in Rust, about 140K lines including tests. The rule I set is that every decision about keys and money is written once, in Rust. That covers validating an address, encrypting the vault, encoding a transaction and signing it. TypeScript handles the DOM, storage and `fetch`; Python runs build and release tooling. The pages are static HTML and CSS with no UI framework. This project is an outcome of more than 9 months of research, design & development.

This note is about the problems that rule ran into, and how I solved them.

## One core, three places to run it

The wallet is four crates. `evm` holds a closed list of networks and the transaction codec. `wallet` works with secrets, the encrypted vault, key derivation, and the Bitcoin, Solana, TRON and XRP code that the apps add on top of the seven EVM networks. `convert` handles swapping and moving between networks without ever holding a key. Finally, `wasm` is a thin wasm-bindgen layer over them.

In the browser, the wallet is WebAssembly. Each operation runs locally in a fresh Web Worker that is terminated when it finishes. Apart from showing the recovery phrase once, the page gets back only public data, ciphertext, or a transaction hash. The apps use [Tauri](https://tauri.app) and link the same crates natively, so the Wasm signer never loads there. The serverless worker is written in Rust as well and compiled to Wasm. It links `evm` and `convert`, so the server checks a transaction with the same code the wallet used to build it.

## One address, and a way back

The first wallet derived a different address for each EVM network. Then 0.001411 ETH sent on Ethereum to its Base address arrived but appeared nowhere. Before launch I changed it to one standard derivation path across every EVM network. Balances stay separate, signatures stay bound to the chosen chain, and the same words recover the address in another compatible wallet.

Recovery also had to work without an AINU account. Creation requires an encrypted backup download and checks three randomly chosen recovery words before persisting the vault. Once loaded, `/recover` works offline from the phrase or the backup and its password. The recovery harness checks both in fresh browsers.

## A network layer without a network

Sending Bitcoin or XRP takes several requests in a fixed order, with a second provider to fall back on. The same core runs under `cargo test`, in browser Wasm and in the app's host. I kept its network logic independent of each environment's transport, without I/O. An operation is a small state machine. It hands out one request at a time and reads each answer before naming the next:

```rust
pub enum Step<T> {
    /// Carry this request and feed the result to [`Operation::resume`].
    Send(HttpRequest),
    /// The operation is over: the chain's answer, or nobody's.
    Done(Result<T, Unavailable>),
}

pub trait Protocol {
    type Provider: 'static;
    type Outcome;

    fn provider_key(provider: &Self::Provider) -> &'static str;
    fn begin(&mut self, provider: &'static Self::Provider) -> Progress<Self::Outcome>;
    fn advance(
        &mut self,
        provider: &'static Self::Provider,
        response: HttpResponse,
    ) -> Result<Progress<Self::Outcome>, Failure>;
}
```

Each chain implements `Protocol`, and one shared engine drives it through a pinned list of providers. The transport only carries bytes, whether it is the app's HTTPS client or a test's list of canned answers.

Writing it this way forced me to state the failover rules. An operation is one provider's view: when a provider fails part-way, the operation starts again from its first request at the next one, because mixing two providers' answers would let a lagging node contradict a leading one. A provider that answers something unwelcome, such as a refused broadcast, is believed, and no second provider is asked for a better answer. And when a broadcast fails in a way where the provider may still have seen it, the wallet records the transfer as pending and does not offer to send it again.

It also kept TLS out of the Rust build. On Apple platforms and Windows the transport is `reqwest` over the operating system's TLS. On Android the same choice would have meant compiling OpenSSL, so a small Kotlin plugin calls `HttpsURLConnection`, and Rust still builds every request and reads every answer.

## Keeping unsafe and panics out

The workspace forbids `unsafe`. Three small desktop crates outside it call macOS and Windows APIs directly. Their unsafe blocks need written safety arguments, enforced by `undocumented_unsafe_blocks = "deny"`.

Since the end of October, the rule is to return errors instead of deliberately panicking. Clippy denies `unwrap_used`, `expect_used`, and the assert family through `disallowed_macros`. These checks catch common panic sources without proving the whole program panic-free. Tests opt back in at each crate root:

```rust
// Tests may unwrap, expect and assert; the code they test may not.
#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_macros)
)]
```

The release profile aborts on panic, so in Wasm a panic takes the whole instance down. The fuzzing harness sits outside the workspace and compiles the Worker's CBOR, WebAuthn and TOTP parsers from their own source files through `#[path]`, so it fuzzes the code that ships. 

## The serverless workers

Serverless's Rust Workers handle sign-in, sessions, market data and signed-transaction submission. I get scaling without managing servers, but requests still have resource limits. Database stores persistent records, and separate objects storage live forecasts.

<blockquote class="twitter-tweet"><p lang="en" dir="ltr">Those who win stay ahead of the market.
Introducing ainu Capital:

💰 A self-custodial wallet.
📊 Monitor market forecasts.
💡 See the signals. Own your decisions.

👇:
https://t.co/eEIRpTk3MW

#AINU #AINUCapital #Crypto #Forecasts #CryptoTrading https://t.co/D41wbtHrel</p>&mdash; Ainu (@ainu_ae) <a href="https://x.com/ainu_ae/status/2122530572480483814?ref_src=twsrc%5Etfw">November 17, 2026</a></blockquote>
<script async src="https://platform.x.com/widgets.js" charset="utf-8"></script>


## Forecast accuracy you can inspect

Our in-house AI model analyzes real-time market signals and forecasts price direction up to one day ahead, with model inference taking tens of milliseconds.
I wanted users to judge a forecast by its record. The Accuracy view counts correct directions out of resolved calls, scoring Instant and Deep predictions separately by coin, horizon and period. It shows the sample count beside the percentage, helping users judge how much weight to give a new prediction.
Forecast outcomes are determined using Chainlink price data. The resolution process could also be adapted to use Binance prices.

## Four apps from one Rust codebase

Writing the interface again in Swift and Kotlin would have meant three interfaces for one developer to keep correct, and in a wallet a mis-rendered address or amount is the worst kind of bug. Tauri 2 let me bundle the web interface into the app and keep the keys in the Rust host. The webview may call 46 commands and nothing else. The shell sits outside the Cargo workspace with its own lockfile, because its webview backend needs system libraries that the workspace's Linux checks do not carry.

Each platform had its own problem:

- **iOS.** The iOS 27 SDK makes the UIScene life cycle mandatory. Declaring the scene manifest got the app launching, but release builds crashed at launch: tao 0.35 handed UIKit a scene configuration it had already released. tao 0.37.1 returns it autoreleased, so I moved the shell to Tauri 2.12.4.
- **Android.** Opening an outside link hung the app. Android asks about navigation on its main thread, and the opener plugin waited there for its Kotlin half, which runs on that same thread. Opening the link from a blocking task fixed it.
- **Windows.** A Store package version has four fields of at most 65,535, and the Store wants the last one to be zero. My build numbers are seconds since 2026-01-01, which do not fit. Windows versions became year, day of year, and the seconds since midnight divided by two, such as `2026.300.15092.0`.
- **Bitcoin, in the apps only.** A legacy signature does not commit to the value it spends, and a segwit signature commits only to its own input's value. An indexer that misstates an input could make the wallet burn the difference as a fee. Before quoting, the wallet reads the transaction behind each legacy input, and behind every input when there are several, and accepts it only if its bytes hash to the expected id.

## Builds that reproduce

The wallet's Wasm is built twice in isolated runs and compared byte for byte before a release may use it, and so is the Worker's artifact. This exposed a platform difference: Cargo includes the compiler's host triple in crate disambiguators, so my Mac's build did not match CI's. Releases are now built and sealed inside a pinned Linux image.

For next and production, a separate signature binds wallet permissions to the exact commit, browser/Wasm and Worker artifacts, configuration and lane. GitHub verifies the artifact on a fresh runner without deployment credentials, stages it at 0% traffic, and checks the served bytes before promotion. A reproducible build still needs permission to become the release.

CI time needed a fix as well. The vault's scrypt is slow on purpose, and at `opt-level = 0` it is far slower. The wallet tests took about 56 CPU-minutes, more than a two-vCPU CI host could finish within its 30-minute limit. Optimizing only `scrypt` and `salsa20` in the test profile brought the suite below two minutes, while keeping debug assertions and overflow checks on.

The apps still have to leave testing. The web wallet is live at [capital.ainu.ae](https://capital.ainu.ae).
