# aimer_arrlist

Utilities for building widget lists.

`list!` creates the `Vec<AnyWidget>` that `Column::children`, `Row::children`,
`Stack::children` and friends accept. Items may have different types; each one is
erased into an `AnyWidget`. For a list of a single type use a plain array or vec!, e.g.
`[Text::new("a"), Text::new("b")]`.

```rust,ignore
use aimer_arrlist::list;

Column::new().children(list![
    Text::new("a"),
    SizedBox::new().height(8),
    Icon::new(),
]);
```
