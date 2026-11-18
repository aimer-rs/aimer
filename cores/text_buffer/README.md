# aimer_text_buffer

A rope: text held as a tree of chunks of a few kilobytes, so that

- an edit anywhere copies the chunk or two it touches, never the text;
- a [`TextSnapshot`] is an O(1) clone that never changes, which is what undo history, background
  layout and styling read while the user keeps typing;
- byte, line and grapheme positions are found in O(log n);
- undo records what was removed and inserted as shared pieces of the tree, under a byte cap.

```
use aimer_text_buffer::TextBuffer;

let mut buffer = TextBuffer::from_text("hello world");
let before = buffer.snapshot();
let delta = buffer.replace(6..11, "there");
assert_eq!(buffer.snapshot().to_string(), "hello there");
assert_eq!(delta.removed().to_string(), "world");
assert_eq!(before.to_string(), "hello world", "a snapshot does not change");
buffer.undo();
assert_eq!(buffer.snapshot().to_string(), "hello world");
```

Offsets are UTF-8 byte offsets. A position that is not a character boundary is a bug in the
caller and panics, as it does for `str`; grapheme boundaries are found with
[`TextSnapshot::floor_grapheme_boundary`] and its relatives.

## Text that is already in memory

`TextSnapshot::from_shared` makes a rope whose chunks are slices of a text held elsewhere (a
paste, a file read whole), so it costs the list of chunks and not the text. The chunks keep the
text alive until the last of them is dropped or edited away; an edit copies the chunks it touches
and shares the rest. `with_replaced_snapshot` does the same for a replacement that is itself a
snapshot, which is how a paste of megabytes goes into a text without being copied. The backing
(`SharedText`) must not change while a snapshot holds it.
