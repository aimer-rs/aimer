## Aimer Custom Paint

`CustomPaint` reserves a box in the layout and lets the application draw into it.
The drawing code is a `CanvasPainter`; it records shapes, text and images through
`aimer_canvas::Canvas` in logical pixels, with the origin at the top-left of the box.

The recording is retained: it is made once for a size and scale, replayed on later
frames, and made again only when the box is resized, the scale changes, or the
widget is rebuilt.
