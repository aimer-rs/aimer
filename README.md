![Aimer Logo](.assets/aimer_rust_ui_framework.png)

<div align="center">

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE-MIT)
[![License: APACHE](https://img.shields.io/badge/License-APACHE-blue.svg)](LICENSE-APACHE)
[![Rust Edition: 2024](https://img.shields.io/badge/Rust-2024%20Edition-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/Platform-macOS%20%7C%20iOS%20%7C%20Android%20%7C%20Web%20%7C%20Linux%20%7C%20Windows-lightgrey.svg)](#platform-support)

**A high-performance, asynchronous, declarative GUI framework for Rust.**

 **A**imer **I**s **M**ade **E**ntirely in **R**ust

</div>

Describe your UI as composable Rust widgets. Aimer lays it out, animates it and draws it on the GPU itself:
no webview, no system widgets, and the same code on every platform.

## Why Aimer

- **Declarative and type-safe.** Fluent builders, `StatelessWidget` / `StatefulWidget` derives and a typed `Router`
  keep UI code short and let the compiler catch mistakes.
- **Built for frame time.** Scoped rebuilds, incremental render sync and allocation-free hot paths, tuned for debug
  builds as well as release.
- **One renderer, everywhere.** `Cupid` is a from-scratch 2D vector and text engine: shaping, rasterization,
  gradients, shadows and glass backdrops, with no HarfBuzz, FreeType or cosmic-text.
- **Async from the ground up.** `Venus` runs futures aligned to the frame, so slow work never blocks a draw.
- **Batteries included.** Navigation and gestures, modals, storage, SVG, Markdown, icons
  and animation ship in the box.

## Hello World

```rust
fn main() {
  AimerApp::start(
    Container::new()
      .child(
        Text::new("Hello World!")
          .text_align(TextAlign::MidCenter)
          .text_style(TextStyle::new().color(Color::WHITE))
      )
  );
}
```

## Platform Support

| Platform       | Graphics Backend            |    Status    |
|:---------------|:----------------------------|:------------:|
| **macOS**      | Metal                       | ✅ Supported |
| **iOS**        | Metal                       | ✅ Supported |
| **Android**    | Vulkan with OpenGL fallback | ✅ Supported |
| **Web (WASM)** | WebGPU with WebGL2 fallback | ✅ Supported |
| **Windows**    | Direct3D 12                 | ✅ Supported |
| **Linux**      | Vulkan with OpenGL fallback | ✅ Supported |

## Built with Aimer

[**RustForum**](https://github.com/Cottons29/rust-forum) is a [**Rust Programming Language Forum**](https://users.rust-lang.org/)'s Client written with Aimer, a real-world example to read, run and learn from.

![Aimer Logo](.assets/rust-forum-screenshot.png)

## AI usage


AI tools assist with Aimer's development, audits, testing, documentation, and
project maintenance. Maintainers direct product design and architecture, review
generated changes, and remain responsible for everything that ships.

Contributors are welcome to use AI tools. You must understand, review, and
validate what you submit, explain your decisions, and address feedback. The
same quality standards apply regardless of how a contribution was produced.

Submit only work you can stand behind. AI-generated code, test results, and
claims need verification before they become part of a contribution.

The final merge are perform by Aimer's team after it have been reviewed.

> This is not AI slop project, anyway if you think it's, We `respect` your mind.

## License

Distributed under the MIT License and APACHE. See [LICENSE-MIT](LICENSE-MIT) and [APACHE](LICENSE-APACHE) for details.
