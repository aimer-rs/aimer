# aimer_scene

The GPU-free scene model of the Cupid renderer: immutable [`draw_cmd`] lists, the
retained [`draw_cmd_v2`] render tree with its damage tracking and render order,
the [`compositor`] that plans groups, and the [`frame`] packets a renderer consumes.

Nothing here talks to a graphics API. A renderer in `aimer_cupid` walks these
structures and drives the pipeline crates, so scene logic can be built and tested
without any backend.
