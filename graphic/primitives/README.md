# aimer_primitives

Small value types shared by every layer of the Cupid renderer: [`Rect`], [`Vec2d`],
[`Rgba8`], [`Mat3`], texture ids and the identity hasher.

The crate has no GPU, font or scene dependencies so that the scene model and each
render pipeline can depend on it without depending on each other.
