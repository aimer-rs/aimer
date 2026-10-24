# AimerMarkdown — Development Guide

Instructions for AI coding assistants and human contributors working on the Aimer codebase.

**Never give up on the right solution.**

---

![image.png](assets/image.png)

## Project Overview

AimerMarkdown is a Rust library that provides a set of tools for render the Markdown file in an Aimer framework.

## Golden Rules

- **Use CodeGraph to understand code.** It is fast and always safe for reading/navigating the codebase. Prefer it before
  opening files blindly.
- **Use the IDE (IDEA/CLion) integration to edit code** when connected — it is the safest, fastest path for refactors
  and renames.
- **Never write "Lazy Senior Dev" code.** Do not merely patch the symptom with spaghetti that other developers will
  curse. Solve the actual problem cleanly.
- **Follow Test Driven Development.** Write the failing test first, then the code that makes it pass.

# Implementation Steps

- [x] Add a public RichText/span widget with mixed-style wrapping and link hit regions.
- [ ] Parse Markdown into an internal document tree.
- [x] Map block nodes to Aimer layout widgets and inline nodes to rich-text spans.
- [ ] Add a configurable theme, link callback, image resolver, and selectable/copyable code blocks.

```
This is empty block

```

# Main Markdown Syntax Reference

````markdown

# Headings

# H1

## H2

### H3

#### H4

##### H5

###### H6

# Emphasis

*italic* or _italic_
**bold** or __bold__
***bold italic***
~~strikethrough~~

# Lists

- Unordered item

* Also unordered

+ Also unordered

1. Ordered item
2. Ordered item

# Task Lists

- [x] Completed task
- [ ] Incomplete task

# Links

[link text](https://example.com)
[link text](https://example.com "optional title")
<https://example.com>  (autolink)

# Images

![alt text](image.jpg)
![alt text](image line here)
![alt text](image.jpg "optional title")

# Blockquotes

> This is a quote
>> Nested quote

# Code

`inline code`

```
fenced code block
```

```python
# fenced code block with syntax highlighting
```

    indented code block (4 spaces)

# Horizontal Rule
---
***
___

# Line Breaks

Paragraph one.

Paragraph two (blank line = new paragraph).

Line one  
Line two (two trailing spaces = line break)

# Tables

| Header 1 | Header 2 |
|----------|----------|
| Cell 1   | Cell 2   |
| Cell 3   | Cell 4   |

# Footnotes

Here is a footnote reference[^1].

[^1]: Footnote text.

# Escaping Characters

\* not italic \*


````

# Headings

# H1

## H2

### H3

#### H4

##### H5

###### H6

# Emphasis

*italic* or _italic_
**bold** or __bold__
***bold italic***
~~strikethrough~~

# Lists

- Unordered item

* Also unordered

+ Also unordered

1. Ordered item
2. Ordered item

# Task Lists

- [x] Completed task
- [ ] Incomplete task

# Links

[link text](https://example.com)

[link text](https://example.com "optional title")

<https://example.com>  (autolink)



`ABCD1234` and `EFGH1234` and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and
`EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and
`EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and
`EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234` and `EFGH1234`and
`EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and
`EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and
`EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and
`EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and
`EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`and
`EFGH1234`and `EFGH1234`and `EFGH1234`and `EFGH1234`

# Blockquotes

> This is a quote
>> Nested quote

# Code

`inline code`

```
fenced code block
```

```python
# fenced code block
print("Hello World")
```

    indented code block (4 spaces)

# Horizontal Rule
---
***
___

# Line Breaks

Paragraph one.

Paragraph two (blank line = new paragraph).

Line one  
Line two (two trailing spaces = line break)

# Tables

| Header 1 | Header 2 |
|----------|----------|
| Cell 1   | Cell 2   |
| Cell 3   | Cell 4   |

# Footnotes

Here is a footnote reference[^1].

[^1]: Footnote text.





# Escaping Characters

\* not italic \*

