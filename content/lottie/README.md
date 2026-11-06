# aimer_lottie

Lottie vector animation playback for Aimer.

The crate loads Lottie (`.json`) animations and plays them as retained vector
content, the same way `aimer_svg` plays static SVG. It is optional: enable the
`lottie` feature on the `aimer` crate to use it as `aimer::lottie`.

## Scope

Aimer plays a documented subset of Lottie rather than the whole format:

- shape layers, transforms, trim paths, fills and strokes;
- keyframed properties with bezier easing.

Anything outside the subset is reported as a diagnostic and skipped instead of
being rendered wrongly. In particular, After Effects expressions are not run by
a JavaScript engine; a small in-crate interpreter handles the common idioms
(`loopOut`, `wiggle`, `Math.*`, simple arithmetic) and every other expression
falls back to the property's plain keyframes.

## Usage

```rust,ignore
use std::sync::Arc;

use aimer::lottie::{Composition, Lottie, LottieScene};

let composition = Composition::parse(include_str!("spinner.json"))?;
let animation = Arc::new(LottieScene::new(composition));

// Share `animation` between widgets; rebuilding never parses again.
let spinner = Lottie::new().width(48.0).animation(animation);
```

## Status

Loads, evaluates and draws: `Composition::parse` reads a document into a typed
model and lists what it skipped, `LottieScene` turns it into an SVG scene that
Cupid draws with per-frame overrides, and the `Lottie` widget plays it with
loop, speed, pause, fixed progress and fit.

Track mattes work for the alpha kind: the matted layer is drawn through the
renderer's coverage mask, which costs two offscreen targets per matted layer
and a full subtree walk on every frame. Inverted and luma mattes are reported
and their layers are left out.

Not done yet: gradients, masks, repeaters, polystars, rounded corners, solid
layers and precomps are not read, expressions are skipped, and only a handful
of files have been checked on screen.
