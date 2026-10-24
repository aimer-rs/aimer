# The Road from Source to AWIR

### Source Code

```
[SourceCode]
Container::new()
    .color(Color::YELLOW)
    .child(Text::new("Hello World!"))
```

## Semantic Graph

```
[Widget IR: semantic graph]
root=node1 generation=1 revision=0
node0: widget=0x351f5f07c36cba52 schema=1.0 key=Some(StableId128([134, 249, 11, 20, 29, 127, 155, 85, 187, 60, 199, 3, 237, 21, 142, 4]))
  property=0x2fae295400f00d82 optional=false value=StringRef(0) string="Hello World!"
node1: widget=0xa95a5cb1843fa837 schema=1.0 key=Some(StableId128([182, 112, 148, 115, 186, 189, 192, 199, 15, 160, 155, 250, 66, 189, 89, 1]))
  property=0x854b302a6145bc81 optional=false value=Rgba(4294902015)
  child=node0
```

## Textual Assembly (Human Readable)

```
[Widget IR: textual assembly]
AWIR 2 0
GENERATION 1
REVISION 0
SECTION TEXT
ROOT node1
node0:
  NODE 0x351f5f07c36cba52 1 0
  KEY 86f90b141d7f9b55bb3cc703ed158e04
  PROP 0x2fae295400f00d82 STRREF string0
  END
node1:
  NODE 0xa95a5cb1843fa837 1 0
  KEY b6709473babdc0c70fa09bfa42bd5901
  PROP 0x854b302a6145bc81 RGBA 0xffff00ff
  CHILD node0
  END
SECTION DATA
string0:
  STRING "Hello World!"
```

## Compact Binary AWIR

```

[Widget IR: compact binary AWIR] bytes=280 nodes=2 properties=2 callbacks=0 children=1
00000000: 41 57 49 52 02 00 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 00 00 00 02 00 00 00
00000020: 02 00 00 00 00 00 00 00 01 00 00 00 01 00 00 00 0c 00 00 00 00 00 00 00 00 00 00 00 18 01 00 00
00000040: 52 ba 6c c3 07 5f 1f 35 01 00 00 00 01 00 00 00 86 f9 0b 14 1d 7f 9b 55 bb 3c c7 03 ed 15 8e 04
00000060: 00 00 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00000080: 37 a8 3f 84 b1 5c 5a a9 01 00 00 00 01 00 00 00 b6 70 94 73 ba bd c0 c7 0f a0 9b fa 42 bd 59 01
000000a0: 01 00 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 00 00 00 00 00 00 00 00 00 00 00
000000c0: 82 0d f0 00 54 29 ae 2f 05 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
```

### First Commission

```
committed generation 1
000000e0: 81 bc 45 61 2a 30 4b 85 04 00 00 00 00 00 00 00 ff 00 ff ff 00 00 00 00 00 00 00 00 00 00 00 00
00000100: 00 00 00 00 00 00 00 00 0c 00 00 00 48 65 6c 6c 6f 20 57 6f 72 6c 64 21
```

## Decoded AWIR

```
[Widget IR: decoded AWIR]
node0: widget=0x351f5f07c36cba52 schema=1.0 key=Some(StableId128([134, 249, 11, 20, 29, 127, 155, 85, 187, 60, 199, 3, 237, 21, 142, 4]))
  property=0x2fae295400f00d82 optional=false value=StringRef(0)
node1: widget=0xa95a5cb1843fa837 schema=1.0 key=Some(StableId128([182, 112, 148, 115, 186, 189, 192, 199, 15, 160, 155, 250, 66, 189, 89, 1]))
  property=0x854b302a6145bc81 optional=false value=Rgba(4294902015)
  child=node0
data strings=1 blobs=0
[Widget IR: schema validation] accepted
[Widget IR: native materialization] ready
```
