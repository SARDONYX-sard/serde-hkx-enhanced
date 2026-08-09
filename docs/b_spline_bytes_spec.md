# Havok Spline Compression Binary Specification

This document specifies the binary layout decoded and encoded by `SplineDecompressor`.

Reference implementation:

- [`hka_spline_decompressor.hpp`](https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp)
- [`hka_spline_decompressor.cpp`](https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp)

---

## 1. Block Layout

A compressed animation block is laid out as follows:

```text
byte offset
    │
    ├── 0x0000 ────────────────────────────────┐
    │                                          │
    │      TransformMask[0]                    │
    │      TransformMask[1]                    │
    │      ...                                 │
    │      TransformMask[num_tracks - 1]       │
    │                                          │
    ├── align(4) ──────────────────────────────│
    │                                          │
    │      TransformTrack[0]                   │
    │      TransformTrack[1]                   │
    │      ...                                 │
    │      TransformTrack[num_tracks - 1]      │
    │                                          │
    ├── align(4) ──────────────────────────────│
    │                                          │
    │      Float-track region                  │
    │                                          │
    └── align(16) ─────────────────────────────┘
```

The mask table always precedes the transform-track data.

Each `TransformMask` occupies exactly 4 bytes.

```text
mask_offset(track) = track * 4
```

The transform-track region begins at:

```text
transform_base = align_up(num_tracks * 4, 4)
```

The actual size of each transform track is variable and is determined by
its mask.

---

# 2. TransformMask

Each transform track begins logically with one 4-byte mask entry.

```text
byte +0
  7            6 5             2 1              0
  ┌─────────────┬───────────────┬───────────────┐
  │    Scale    │    Rotation   │    Position   │
  │     2 bits  │     4 bits    │     2 bits    │
  └─────────────┴───────────────┴───────────────┘

byte +1
  7 6 5 4 3 2 1 0
  ┌────────────────┐
  │ position_types │
  └────────────────┘

byte +2
  7 6 5 4 3 2 1 0
  ┌────────────────┐
  │ rotation_types │
  └────────────────┘

byte +3
  7 6 5 4 3 2 1 0
  ┌────────────────┐
  │   scale_types  │
  └────────────────┘
```

The four bytes are:

```text
+0  quantization_types
+1  position_types
+2  rotation_types
+3  scale_types
```

## `quantization_types`

```text
bit 7    6 5   4   3   2 1    0
    ┌─────┬─────────────┬─────┐
    │Scale│  Rotation   │ Pos │
    │  2  │      4      │  2  │
    └─────┴─────────────┴─────┘
```

| Bits | Field    | Encoding                |
| ---- | -------- | ----------------------- |
| 1:0  | Position | scalar quantization     |
| 5:2  | Rotation | quaternion quantization |
| 7:6  | Scale    | scalar quantization     |

Position:

```text
00 = Bit8
01 = Bit16
10 = invalid
11 = invalid
```

Rotation:

```text
bits 5:2 + 2

2 = Bit32
3 = Bit40
4 = Bit48
5 = Bit24
6 = Bit16Quat
7 = Uncompressed
```

Scale:

```text
00 = Bit8
01 = Bit16
10 = invalid
11 = invalid
```

---

# 3. Transform Type Flags

The three component flag bytes use the following bit layout.

## 3.1 Position

```text
bit:  7     6     5     4     3     2     1     0
     ┌─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┐
     │     │     │ SPL │ SPL │ SPL │ STA │ STA │ STA │
     │     │     │  Z  │  Y  │  X  │  z  │  y  │  x  │
     └─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┘
```

More precisely:

```text
bit 0 = STATIC_X
bit 1 = STATIC_Y
bit 2 = STATIC_Z

bit 4 = SPLINE_X
bit 5 = SPLINE_Y
bit 6 = SPLINE_Z

bits 3, 7 = reserved
```

A component is interpreted in this order:

```text
STATIC -> SPLINE -> IDENTITY
```

Therefore:

```text
static bit set
    => Static

static bit clear, spline bit set
    => Dynamic

both clear
    => Identity
```

---

## 3.2 Rotation

Rotation uses all four quaternion components.

```text
bit:  7     6     5     4     3     2     1     0
     ┌─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┐
     │ W   │ Z   │ Y   │ X   │ W   │ Z   │ Y   │ X   │
     │ SPL │ SPL │ SPL │ SPL │ STA │ STA │ STA │ STA │
     └─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┘
```

```text
bit 0 = STATIC_X
bit 1 = STATIC_Y
bit 2 = STATIC_Z
bit 3 = STATIC_W

bit 4 = SPLINE_X
bit 5 = SPLINE_Y
bit 6 = SPLINE_Z
bit 7 = SPLINE_W
```

The rotation track is:

```text
rotation_types & 0xf0 != 0
    => Dynamic

rotation_types & 0x0f != 0
    => Static

otherwise
    => Identity
```

---

## 3.3 Scale

Scale uses the same layout as Position.

```text
bit:  7     6     5     4     3     2     1     0
     ┌─────┬─────┬─────┬─────┬─────┬─────┬─────┬─────┐
     │     │     │ SPL │ SPL │ SPL │     │     │     │
     │     │     │  Z  │  Y  │  X  │     │     │     │
     └─────┴─────┴─────┴─────┴─────┴─────┴─────┴─────┘
```

```text
bit 0 = STATIC_X
bit 1 = STATIC_Y
bit 2 = STATIC_Z

bit 4 = SPLINE_X
bit 5 = SPLINE_Y
bit 6 = SPLINE_Z

bits 3, 7 = reserved
```

---

# 4. Transform Track

A transform track consists of three sections in this exact order:

```text
TransformTrack
    │
    ├── Position
    ├── align(4)
    │
    ├── Rotation
    ├── align(4)
    │
    ├── Scale
    └── align(4)
```

The decoder does not store fixed-size structures for these sections.

Their size is determined entirely by the corresponding flags and
quantization type.

---

# 5. Static Vector Track

A static position or scale component contains one `f32`.

There is no per-component header.

For a vector:

```text
Position/Scale
    │
    ├── if STATIC_X: f32
    ├── if STATIC_Y: f32
    └── if STATIC_Z: f32
```

The values are stored consecutively.

Example:

```text
STATIC_X | STATIC_Z

offset +0  : X f32
offset +4  : Z f32
```

If no static component is present:

```text
size = 0
```

The section is followed by 4-byte alignment.

---

# 6. Dynamic Vector Track

A dynamic position or scale section begins with:

```text
offset +0  u16  num_items
offset +2  u8   degree
offset +3  u8   knot[0]
offset +4  u8   knot[1]
...
```

The number of knot bytes is:

```text
num_items + degree + 2
```

Thus the header and knot vector occupy:

```text
2 + 1 + (num_items + degree + 2)
```

bytes before alignment.

The complete section is:

```text
┌──────────────────────────────────────────────┐
│ u16 num_items                                │ +0
├──────────────────────────────────────────────┤
│ u8 degree                                    │ +2
├──────────────────────────────────────────────┤
│ u8 knot[0]                                   │ +3
│ u8 knot[1]                                   │ +4
│ ...                                          │
│ u8 knot[num_items + degree + 1]             │
├──────────────────────────────────────────────┤
│ padding                                      │
├──────────────────────────────────────────────┤
│ bounds                                       │
├──────────────────────────────────────────────┤
│ quantized control points                     │
└──────────────────────────────────────────────┘
```

The section is aligned to 4 bytes before the bounds.

---

# 7. Dynamic Vector Bounds

Only dynamic axes have bounds.

For each dynamic axis, exactly two `f32` values are stored:

```text
f32 minimum
f32 maximum
```

The order is the axis order:

```text
X
Y
Z
```

but only axes whose corresponding spline flag is set are present.

For example:

```text
SPLINE_X | SPLINE_Z

+0   f32 X.minimum
+4   f32 X.maximum
+8   f32 Z.minimum
+12  f32 Z.maximum
```

A non-dynamic axis contributes no bound bytes.

---

# 8. Dynamic Vector Control Points

There are:

```text
num_items + 1
```

control points.

Each control point contains only the active dynamic axes.

The order is always:

```text
X
Y
Z
```

with inactive axes omitted.

For example:

```text
SPLINE_X | SPLINE_Z
```

produces:

```text
point[0]:
    X
    Z

point[1]:
    X
    Z

...

point[num_items]:
    X
    Z
```

---

# 9. Scalar Quantization

## 9.1 Bit8

One control-point value occupies exactly one byte:

```text
u8 value
```

The decoded value is:

```text
t = value / 255.0

decoded = min + (max - min) * t
```

Therefore:

```text
encoded range = 0 ..= 255
```

---

## 9.2 Bit16

One control-point value occupies two bytes:

```text
offset +0  u16 value
offset +2  padding
```

The integer is little-endian.

The decoded value is:

```text
t = value / 65535.0

decoded = min + (max - min) * t
```

The two-byte padding is part of the binary layout.

Therefore one Bit16 scalar consumes:

```text
4 bytes
```

in the control-point stream.

---

# 10. Dynamic Quaternion Track

A dynamic quaternion section begins with the same spline header:

```text
offset +0  u16 num_items
offset +2  u8  degree
offset +3  u8  knot[0]
...
```

The number of knots is:

```text
num_items + degree + 2
```

After the knot vector, the stream is aligned according to the quaternion
quantization format.

The section then contains:

```text
num_items + 1
```

quaternion control points.

There are no scalar bounds for quaternion tracks.

---

# 11. Static Quaternion Track

A static quaternion contains exactly one encoded quaternion.

Before the quaternion value, the stream is aligned according to the
rotation quantization format.

```text
padding
encoded quaternion
```

The encoded quaternion size depends on the quantization type.

---

# 12. Quaternion Quantization Sizes

| Format              |     Size |
| ------------------- | -------: |
| Bit32 / Polar32     |  4 bytes |
| Bit40 / ThreeComp40 |  5 bytes |
| Bit48 / ThreeComp48 |  6 bytes |
| Bit24               |  3 bytes |
| Bit16Quat           |  2 bytes |
| Uncompressed        | 16 bytes |

The rotation stream must satisfy the alignment requirement of the selected
encoding before the first quaternion value.

---

# 13. ThreeComp40

Each quaternion occupies 5 bytes.

The logical 40-bit value is:

```text
bit  39                         0
     ┌───┬────┬─────────────────┐
     │ S │ Q  │   three values  │
     └───┴────┴─────────────────┘
       1   2          36
```

The three stored components occupy 12 bits each:

```text
bits  0..11   component 0
bits 12..23   component 1
bits 24..35   component 2
bits 36..37   omitted component index
bit      38   omitted component sign
bit      39   reserved
```

The resulting 40-bit value is written little-endian as 5 bytes.

---

# 14. ThreeComp48

Each quaternion occupies 6 bytes.

The three stored components occupy 15 bits each.

```text
word 0:
    bits 0..14   component 0
    bit  15      omitted index bit 0

word 1:
    bits 0..14   component 1
    bit  14      omitted index bit 1

word 2:
    bits 0..14   component 2
    bit  15      omitted-component sign
```

Each word is little-endian.

The omitted quaternion component is reconstructed by the decoder.

---

# 15. Polar32

A Polar32 quaternion occupies exactly 4 bytes.

```text
bit 31                         0
     ┌──┬──┬──┬──┬──────────────┐
     │W │Z │Y │X │  polar data  │
     └──┴──┴──┴──┴──────────────┘
      1  1  1  1       28
```

The low 18 bits contain the polar/angular value.

The next 10 bits contain the radial value.

The upper four bits contain component signs.

```text
bits  0..17   phi/theta data
bits 18..27   radial data
bit      28   sign X
bit      29   sign Y
bit      30   sign Z
bit      31   sign W
```

The complete word is little-endian.

---

# 16. Uncompressed Quaternion

An uncompressed quaternion occupies 16 bytes.

```text
offset +0   f32 X
offset +4   f32 Y
offset +8   f32 Z
offset +12  f32 W
```

All values are little-endian IEEE-754 `f32`.

---

# 17. Alignment

Alignment is performed by inserting zero bytes.

For an alignment `N`:

```text
aligned_offset = (offset + N - 1) & !(N - 1)
```

The following alignments apply:

```text
TransformMask table       4 bytes
Position section           4 bytes
Rotation section           quantization-specific
Scale section              4 bytes
Float-track region        16 bytes
```

Padding bytes do not represent semantic data.

---

# 18. Float Track Region

The transform-track region is followed by the float-track region.

The decoder skips:

```text
num_float_tracks
```

bytes at the beginning of this region.

The float-track data itself is outside the transform spline structures
described by this document.

---

# 19. Block Offsets

The animation contains a block-offset table.

For block `i`:

```text
block_start = block_offsets[i]
```

The next block begins at:

```text
block_offsets[i + 1]
```

For the final block, the containing animation data determines its end.

The offsets are byte offsets from the beginning of the compressed animation
data.

---

# 20. Example: One Dynamic Position Track

Assume:

```text
num_items = 2
degree    = 1

position_types = SPLINE_X
quantization   = Bit8
```

Then:

```text
number of control points
    = num_items + 1
    = 3

number of knots
    = num_items + degree + 2
    = 5
```

The section is:

```text
offset
+00  u16  num_items = 2
+02  u8   degree    = 1
+03  u8   knot[0]
+04  u8   knot[1]
+05  u8   knot[2]
+06  u8   knot[3]
+07  u8   knot[4]

+08  f32  X.minimum
+0C  f32  X.maximum

+10  u8   control_point[0].X
+11  u8   control_point[1].X
+12  u8   control_point[2].X

+13  padding
```

The section ends at the next 4-byte boundary.

---

# 21. Example: Static Position XYZ

Assume:

```text
STATIC_X | STATIC_Y | STATIC_Z
```

The section is exactly:

```text
offset
+00  f32 X
+04  f32 Y
+08  f32 Z
```

Then:

```text
next_offset = align_up(offset + 12, 4)
            = offset + 12
```

---

# 22. Example: Dynamic Position XZ with Bit16

Assume:

```text
SPLINE_X | SPLINE_Z
quantization = Bit16
num_items = 1
degree = 1
```

There are two control points.

The layout is:

```text
offset
+00  u16 num_items
+02  u8  degree

+03  u8  knot[0]
+04  u8  knot[1]
+05  u8  knot[2]
+06  u8  knot[3]

+07  padding
```

After 4-byte alignment:

```text
+08  f32 X.minimum
+0C  f32 X.maximum
+10  f32 Z.minimum
+14  f32 Z.maximum
```

Control points:

```text
+18  u16 X[0]
+1A  padding

+1C  u16 Z[0]
+1E  padding

+20  u16 X[1]
+22  padding

+24  u16 Z[1]
+26  padding
```

Finally:

```text
+28  end of section
```

The next transform component starts after the required 4-byte alignment.

---

# 23. Identity Tracks

An identity component has no stored value.

For a vector component:

```text
STATIC bit = 0
SPLINE bit = 0
```

means:

```text
stored size = 0
```

The decoder supplies the component's default value.

For rotation:

```text
rotation_types & 0xff == 0
```

means the rotation is an identity quaternion.

No quaternion bytes are stored.

---

# 24. Decoder Invariants

The decoder must validate all size-derived accesses before indexing.

In particular:

```text
num_items + 1
num_items + degree + 2
```

must not be allowed to overflow `usize`.

All reads of:

```text
knots
bounds
control points
quaternion values
```

must be bounded by the remaining input length.

A malformed block must return `SplineDecompressError` rather than panic.

---

# 25. Encoder Semantics

The encoder is not required to reproduce the original compressed byte
stream.

Encoding performs a valid representation of the decoded spline data under
the selected quantization and spline representation.

Consequently:

```text
decode(encode(decoded))
```

is expected to reproduce the represented animation within the precision
allowed by the selected quantization format.

It is not expected that:

```text
encode(decode(original)) == original
```

at the byte level.
