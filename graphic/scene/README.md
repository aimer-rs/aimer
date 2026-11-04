# aimer_scene

The GPU-free scene model of the Cupid renderer: the [`draw_cmd`] commands
elements record and the retained render tree that holds them, with its damage
tracking and render order, the [`compositor`] that plans groups, and the
[`frame`] packets a renderer consumes.

[`draw_cmd`] is the only drawing API. The renderer replays it lowered into a
flat internal stream (`render_list`), which is framework-internal and hidden
from the documentation.

Nothing here talks to a graphics API. A renderer in `aimer_cupid` walks these
structures and drives the pipeline crates, so scene logic can be built and tested
without any backend.
