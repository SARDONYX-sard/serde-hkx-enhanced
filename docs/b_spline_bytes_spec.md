# Havok Spline Compression Binary Specification

This document specifies the binary layout decoded by `SplineDecompressor`.

The binary layout is defined in terms of:

- exact field types and sizes;
- byte offsets;
- element stride;
- section alignment;
- padding;
- byte order.

Semantic decoding rules are described after the physical layout.

Reference implementation:

- [`hka_spline_decompressor.hpp`](https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.hpp)
- [`hka_spline_decompressor.cpp`](https://github.com/PredatorCZ/HavokLib/blob/master/source/hka_spline_decompressor.cpp)

---

# 1. Binary Layout Overview

A compressed spline animation block has the following physical layout:

```text
block_start
    │
    ▼
┌────────────────────────────────────────────────────────────────────┐
│ TransformMask table                                                │
│                                                                    │
│ TransformMask[0]                                                   │
│ TransformMask[1]                                                   │
│ ...                                                                │
│ TransformMask[num_tracks - 1]                                      │
│                                                                    │
│ element size : 4 bytes                                             │
│ total size   : num_tracks × 4 bytes                                │
│ alignment : none                                                   │
└────────────────────────────────────────────────────────────────────┘
    │
    │ immediately followed by
    ▼
┌────────────────────────────────────────────────────────────────────┐
│ Float-track region                                                 │
│                                                                    │
│ raw float-track data                                               │
│                                                                    │
│ size      : num_float_tracks bytes                                 │
│ alignment    : 4 bytes                                             │
└────────────────────────────────────────────────────────────────────┘
    │
    │ align_up(current_offset, 4)
    │
    │ 0..3 bytes padding
    ▼
┌────────────────────────────────────────────────────────────────────┐
│ TransformTrack[0]                                                  │
│                                                                    │
│ Position                                                           │
│ align(4)                                                           │
│ Rotation                                                           │
│ align(4)                                                           │
│ Scale                                                              │
│ align(4)                                                           │
└────────────────────────────────────────────────────────────────────┘
    │
    ▼
┌────────────────────────────────────────────────────────────────────┐
│ TransformTrack[1]                                                  │
│                                                                    │
│ ...                                                                │
└────────────────────────────────────────────────────────────────────┘
    │
    ▼
                              ...
    │
    ▼
┌────────────────────────────────────────────────────────────────────┐
│ TransformTrack[num_tracks - 1]                                     │
└────────────────────────────────────────────────────────────────────┘
```

The transform-track base offset is:

```text
mask_size  = num_tracks × 4
float_size = num_float_tracks

transform_base =
    align_up(mask_size + float_size, 4)
```

There is **no 16-byte alignment between the float-track region and the
transform-track region**.

---

# 2. Primitive Types

Unless otherwise specified, all integer and floating-point values are
little-endian.

| Type   |     Size | Alignment | Description                       |
| ------ | -------: | --------: | --------------------------------- |
| `u8`   |   1 byte |         1 | unsigned 8-bit integer            |
| `u16`  |  2 bytes |         2 | unsigned 16-bit integer           |
| `u32`  |  4 bytes |         4 | unsigned 32-bit integer           |
| `f32`  |  4 bytes |         4 | IEEE-754 single-precision float   |
| `u40`  |  5 bytes |         1 | 40-bit packed integer             |
| `u48`  |  6 bytes |         1 | 48-bit packed integer             |
| `u128` | 16 bytes |        16 | 128-bit value / four `f32` values |

`u40` and `u48` are logical packed values. They are not native C/C++
integer types.

Padding bytes are not semantic values.

---

# 3. Alignment

For an alignment `N`, where `N` is a power of two:

```text
align_up(offset, N) =
    (offset + N - 1) & ~(N - 1)
```

Padding size is:

```text
padding =
    align_up(offset, N) - offset
```

The relevant alignments are:

```text
TransformMask              4 bytes
Float-track region         no internal alignment
TransformTrack start      4 bytes
Position section           4 bytes
Rotation section           format-dependent / 4-byte boundary
Scale section              4 bytes
```

A padding region is explicitly represented as:

```text
┌──────────────────────────────┐
│ padding: u8 × N              │
└──────────────────────────────┘
```

Padding bytes have no semantic meaning.

---

# 4. TransformMask

Each transform track has exactly one `TransformMask`.

```text
TransformMask
size      = 4 bytes
alignment = 4 bytes
```

Physical layout:

```text
offset +0
┌────────────────────────────────────────────────────────────────────┐
│ u8 quantization_types                                              │
└────────────────────────────────────────────────────────────────────┘

offset +1
┌────────────────────────────────────────────────────────────────────┐
│ u8 position_types                                                  │
└────────────────────────────────────────────────────────────────────┘

offset +2
┌────────────────────────────────────────────────────────────────────┐
│ u8 rotation_types                                                  │
└────────────────────────────────────────────────────────────────────┘

offset +3
┌────────────────────────────────────────────────────────────────────┐
│ u8 scale_types                                                     │
└────────────────────────────────────────────────────────────────────┘
```

Therefore:

```text
sizeof(TransformMask) = 4
```

The mask table is:

```text
TransformMask[0]       // 4 bytes
TransformMask[1]       // 4 bytes
...
TransformMask[N - 1]   // 4 bytes
```

and occupies:

```text
N × 4 bytes
```

---

# 5. TransformMask Quantization Types

`quantization_types` is one `u8`.

```text
u8 quantization_types
```

Bit layout:

```text
bit
 7 6 | 5 4 3 2 | 1 0
─────┼─────────┼─────
Scale   Rotation  Position
  2        4         2
```

```text
bits 0..1 = position quantization
bits 2..5 = rotation quantization
bits 6..7 = scale quantization
```

## Position and Scale

```text
00 = Bit8
01 = Bit16
10 = invalid
11 = invalid
```

## Rotation

The rotation value is encoded as:

```text
rotation_type = (quantization_types >> 2) & 0x0f
```

The supported encodings are:

```text
2 = Bit32 / Polar32
3 = Bit40 / ThreeComp40
4 = Bit48 / ThreeComp48
5 = Bit24
6 = Bit16Quat
7 = Uncompressed
```

---

# 6. Transform Track

A transform track contains:

```text
TransformTrack
    │
    ├── Position
    ├── align(4)
    ├── Rotation
    ├── align(4)
    ├── Scale
    └── align(4)
```

The size is variable.

```text
sizeof(TransformTrack) = variable
alignment               = 4
```

The size of each section is determined by its corresponding type flags.

---

# 7. Position Types

`position_types` is one `u8`.

```text
u8 position_types
```

Bit layout:

```text
bit
 7  6  5  4 | 3 | 2  1  0
────┬───────┼───┼────────
    │       │   │
    │       │   └─ STATIC_X/Y/Z
    │       │
    │       └──── reserved
    │
    └───────────── SPLINE_X/Y/Z
```

Precisely:

```text
bit 0 = STATIC_X
bit 1 = STATIC_Y
bit 2 = STATIC_Z

bit 3 = reserved

bit 4 = SPLINE_X
bit 5 = SPLINE_Y
bit 6 = SPLINE_Z

bit 7 = reserved
```

For each component:

```text
STATIC set
    => static value

STATIC clear + SPLINE set
    => dynamic spline

STATIC clear + SPLINE clear
    => identity
```

Static takes precedence over spline.

---

# 8. Static Position

A static position component is one `f32`.

```text
┌──────────────────────────────┐
│ f32 component                │
│ 4 bytes                      │
└──────────────────────────────┘
```

The components are stored in X/Y/Z order.

Example:

```text
STATIC_X | STATIC_Z

offset +0    f32 X
offset +4    f32 Z
```

Size:

```text
static_position_size =
    static_axis_count × 4
```

Alignment:

```text
4 bytes
```

---

# 9. Dynamic Position

A dynamic position begins with:

```text
u16 num_items
u8  degree
u8  knot[0]
...
```

The knot count is:

```text
knot_count =
    num_items + degree + 2
```

Physical layout:

```text
offset +0
┌──────────────────────────────┐
│ u16 num_items                │
│ 2 bytes                      │
└──────────────────────────────┘

offset +2
┌──────────────────────────────┐
│ u8 degree                    │
│ 1 byte                       │
└──────────────────────────────┘

offset +3
┌──────────────────────────────┐
│ u8 knot[0]                   │
│ 1 byte                       │
├──────────────────────────────┤
│ u8 knot[1]                   │
│ 1 byte                       │
├──────────────────────────────┤
│ ...                          │
├──────────────────────────────┤
│ u8 knot[knot_count - 1]      │
│ 1 byte                       │
└──────────────────────────────┘
```

Header size:

```text
2 + 1 + knot_count
```

The section is then aligned to 4 bytes before its bounds.

---

# 10. Dynamic Position Bounds

Each active spline axis has two `f32` values:

```text
f32 minimum
f32 maximum
```

Each axis therefore consumes:

```text
8 bytes
```

Bounds are stored in X/Y/Z order, with inactive axes omitted.

Example:

```text
SPLINE_X | SPLINE_Z

┌──────────────────────────────┐
│ f32 X.minimum                │ 4 bytes
├──────────────────────────────┤
│ f32 X.maximum                │ 4 bytes
├──────────────────────────────┤
│ f32 Z.minimum                │ 4 bytes
├──────────────────────────────┤
│ f32 Z.maximum                │ 4 bytes
└──────────────────────────────┘

size = 16 bytes
alignment = 4
```

---

# 11. Dynamic Position Control Points

The number of control points is:

```text
control_point_count = num_items + 1
```

Only dynamic axes are stored.

Axis order:

```text
X
Y
Z
```

Example:

```text
SPLINE_X | SPLINE_Z
```

Each control point contains:

```text
X
Z
```

Therefore:

```text
point[0]
point[1]
...
point[num_items]
```

The physical element stride depends on the scalar quantization format.

---

# 12. Scalar Quantization

## 12.1 Bit8

A Bit8 scalar is:

```text
u8 value
```

Element size:

```text
1 byte
```

The normalized value is:

```text
t = value / 255.0
```

The decoded value is:

```text
decoded =
    minimum + (maximum - minimum) × t
```

---

## 12.2 Bit16

A Bit16 scalar has a **4-byte element stride**.

```text
┌──────────────────────────────┬──────────────────────────────┐
│ u16 value                    │ u16 reserved                 │
│ 2 bytes                      │ 2 bytes                      │
└──────────────────────────────┴──────────────────────────────┘
             4-byte element stride
```

The first `u16` is the quantized value.

The second `u16` is not a scalar value.

Therefore:

```text
value size   = 2 bytes
element size = 4 bytes
alignment    = 2 bytes for the u16 value
stride       = 4 bytes
```

The normalized value is:

```text
t = value / 65535.0
```

The decoded value is:

```text
decoded =
    minimum + (maximum - minimum) × t
```

---

# 13. Dynamic Scale

Scale uses the same physical layout as Position.

```text
Scale
    │
    ├── static components
    │
    └── dynamic components
         ├── spline header
         ├── align(4)
         ├── bounds
         └── control points
```

The axis order is:

```text
X
Y
Z
```

---

# 14. Rotation Types

`rotation_types` is one `u8`.

```text
u8 rotation_types
```

Bit layout:

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

Rotation has four components:

```text
X
Y
Z
W
```

The component interpretation is:

```text
STATIC set
    => static quaternion component

STATIC clear + SPLINE set
    => dynamic quaternion component

STATIC clear + SPLINE clear
    => identity/default component
```

---

# 15. Dynamic Quaternion

A dynamic quaternion uses the spline header:

```text
u16 num_items
u8  degree
u8  knot[knot_count]
```

where:

```text
knot_count =
    num_items + degree + 2
```

The number of quaternion control points is:

```text
num_items + 1
```

There are no scalar minimum/maximum bounds for quaternion components.

Each control point is encoded using the selected quaternion quantization
format.

---

# 16. Quaternion Encoding Sizes

| Encoding            | Physical size | Logical type |
| ------------------- | ------------: | ------------ |
| Bit32 / Polar32     |       4 bytes | `u32`        |
| Bit40 / ThreeComp40 |       5 bytes | `u40`        |
| Bit48 / ThreeComp48 |       6 bytes | `u48`        |
| Bit24               |       3 bytes | `u24`        |
| Bit16Quat           |       2 bytes | `u16`        |
| Uncompressed        |      16 bytes | `u128`       |

The encoded quaternion is read according to the selected quantization
format.

The byte alignment required before the encoded quaternion is determined by
the decoder's format-specific packing rules.

---

# 17. Static Quaternion

A static quaternion contains exactly one encoded quaternion.

```text
padding
encoded quaternion
```

The encoded quaternion size is:

```text
Bit32         = 4 bytes
Bit40         = 5 bytes
Bit48         = 6 bytes
Bit24         = 3 bytes
Bit16Quat     = 2 bytes
Uncompressed  = 16 bytes
```

For an uncompressed quaternion:

```text
┌──────────────────────────────┐
│ f32 X                       │ 4 bytes
├──────────────────────────────┤
│ f32 Y                       │ 4 bytes
├──────────────────────────────┤
│ f32 Z                       │ 4 bytes
├──────────────────────────────┤
│ f32 W                       │ 4 bytes
└──────────────────────────────┘

size = 16 bytes
```

Logical representation:

```text
u128
```

Physical interpretation:

```text
[f32 X][f32 Y][f32 Z][f32 W]
```

---

# 18. ThreeComp40

Physical size:

```text
5 bytes
```

Logical representation:

```text
u40
```

The 40-bit value is stored little-endian:

```text
byte 0       byte 1       byte 2       byte 3       byte 4
┌──────────┬──────────┬──────────┬──────────┬──────────┐
│ bits 0-7 │ bits 8-15│bits16-23 │bits24-31 │bits32-39 │
└──────────┴──────────┴──────────┴──────────┴──────────┘
```

The exact component extraction is performed by the decoder.

---

# 19. ThreeComp48

Physical size:

```text
6 bytes
```

Logical representation:

```text
u48
```

The value is stored as three little-endian `u16` words:

```text
word 0     word 1     word 2
 2 bytes    2 bytes    2 bytes
```

Total:

```text
3 × sizeof(u16)
    = 6 bytes
```

The decoder extracts the three stored quaternion components and reconstructs
the omitted component.

---

# 20. Polar32

Physical size:

```text
4 bytes
```

Logical representation:

```text
u32
```

The complete value is read as one little-endian `u32`.

The bit fields are interpreted by the Polar32 decoder.

---

# 21. Uncompressed Quaternion

An uncompressed quaternion is four consecutive `f32` values:

```text
offset +0
┌──────────────────────────────┐
│ f32 X                        │
└──────────────────────────────┘

offset +4
┌──────────────────────────────┐
│ f32 Y                        │
└──────────────────────────────┘

offset +8
┌──────────────────────────────┐
│ f32 Z                        │
└──────────────────────────────┘

offset +12
┌──────────────────────────────┐
│ f32 W                        │
└──────────────────────────────┘
```

Total:

```text
size      = 16 bytes
alignment = 4 bytes
```

---

# 22. TransformTrack Physical Layout

For every transform track:

```text
TransformTrack
│
├── Position
│
├── padding to 4-byte boundary
│
├── Rotation
│
├── padding to 4-byte boundary
│
├── Scale
│
└── padding to 4-byte boundary
```

The complete track size is variable.

A conceptual layout is:

```text
track_start
    │
    ▼
┌──────────────────────────────┐
│ Position                     │
│ variable                     │
└──────────────────────────────┘
    │
    │ align(4)
    ▼
┌──────────────────────────────┐
│ Rotation                     │
│ variable                     │
└──────────────────────────────┘
    │
    │ align(4)
    ▼
┌──────────────────────────────┐
│ Scale                        │
│ variable                     │
└──────────────────────────────┘
    │
    │ align(4)
    ▼
track_end
```

---

# 23. Identity Components

An identity component has no physical representation.

For Position and Scale:

```text
STATIC bit = 0
SPLINE bit = 0
```

means:

```text
stored size = 0
```

For Rotation, an absent component does not consume quaternion storage.

The decoder supplies the default identity value.

---

# 24. Complete Block Layout

The complete physical layout can therefore be summarized as:

```text
┌────────────────────────────────────────────────────────────────────┐
│ TransformMask[0]                       4 bytes                     │
├────────────────────────────────────────────────────────────────────┤
│ TransformMask[1]                       4 bytes                     │
├────────────────────────────────────────────────────────────────────┤
│ ...                                                                │
├────────────────────────────────────────────────────────────────────┤
│ TransformMask[num_tracks - 1]          4 bytes                     │
├────────────────────────────────────────────────────────────────────┤
│ Float-track region                     num_float_tracks bytes      │
├────────────────────────────────────────────────────────────────────┤
│ padding                                0..3 bytes                  │
│                                         align(4)                   │
├────────────────────────────────────────────────────────────────────┤
│ TransformTrack[0]                      variable                    │
├────────────────────────────────────────────────────────────────────┤
│ TransformTrack[1]                      variable                    │
├────────────────────────────────────────────────────────────────────┤
│ ...                                                                │
├────────────────────────────────────────────────────────────────────┤
│ TransformTrack[num_tracks - 1]         variable                    │
└────────────────────────────────────────────────────────────────────┘
```

The first transform track begins at:

```text
transform_base =
    align_up(
        num_tracks × sizeof(TransformMask)
        + num_float_tracks,
        4
    )
```

Since:

```text
sizeof(TransformMask) = 4
```

this becomes:

```text
transform_base =
    align_up(
        num_tracks × 4
        + num_float_tracks,
        4
    )
```

---

# 25. Size and Stride Rules

The following distinction is important.

## Size

The physical number of bytes occupied by a value.

Example:

```text
Bit16 value size = 2 bytes
```

## Stride

The distance from one element to the next.

Example:

```text
Bit16 scalar:

value size    = 2 bytes
element stride = 4 bytes
```

Therefore:

```text
control_point[i + 1]
    = control_point[i] + 4 bytes
```

for a Bit16 scalar.

## Section size

The complete physical size of a section, including required padding.

For example:

```text
section_size =
    data_size
    + trailing_alignment_padding
```

This distinction must be preserved throughout the specification.

---

# 26. Decoder Safety Requirements

All size calculations must be checked before converting them to `usize`
or using them for indexing.

In particular:

```text
num_items + 1
num_items + degree + 2
num_tracks × sizeof(TransformMask)
```

must not overflow.

All reads of:

```text
TransformMask
Float-track data
knots
bounds
control points
quaternion values
padding
```

must remain within the input buffer.

Malformed input must return `SplineDecompressError`.

The decoder must not panic because of malformed binary input.

---

# 27. Encoder Semantics

The encoder is not required to reproduce the original compressed byte
stream.

A valid encoder only needs to produce a representation that decodes to the
represented animation within the precision allowed by the selected
quantization format.

Therefore:

```text
decode(encode(decoded))
```

should reproduce the represented animation within the expected quantization
error.

The following is not required:

```text
encode(decode(original)) == original
```

at the byte level.
