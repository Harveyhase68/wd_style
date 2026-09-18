# wd_style — Read a WinDev control's caption/icon layout (and its `..Style` blob)

A tiny, dependency-free Rust DLL for WinDev/WLanguage that answers a question the
official API cannot:

> *"Where is the caption and where is the icon on this control — Top / Bottom /
> Left / Right / Centered, image left/right/top/bottom, or no caption at all?"*

There is **no documented WLanguage property** for this. This project provides **two
independent ways** to get the answer, plus helpers to decompress and fingerprint the
undocumented binary `..Style` blob.

Works with 32-bit and 64-bit WinDev applications. No VC++ Redistributable required
(static CRT). Tested against WinDev 2024/2025 projects.

---

## Two ways to get caption/icon position

| | **A. From the rendered screenshot** (recommended) | **B. From the `..Style` blob** (legacy) |
|---|---|---|
| Function | `WDLayoutAnalyzeBMP2` / `WDLayoutAnalyzeRaw2` | `WDStyleCaptionPosition` |
| Input | a cropped image of the control | the control's `..Style` buffer |
| How | image analysis of what's actually drawn | signature search in the decompressed blob |
| Coverage | **any control, any style sheet, any theme, any WinDev version** | only button style sheets whose signatures are known |
| Blind spots | needs the control to be visible/renderable | returns *inherited* (14) for unknown style sheets / other control types (checkboxes, …) |

**Recommendation:** use **A** (screenshot) for position/layout. It sidesteps the
scrambled blob format entirely and generalizes to every control. **B** still works and
is exported unchanged, but it only recognizes the button style sheets it has
signatures for — on a different style sheet or a checkbox it returns `14` (inherited).

> Migrating from B to A: your old call
> `CallDLL32(sDLL, "WDStyleCaptionPosition", &buf, Length(buf))` **still compiles and
> runs** — the function was not removed. It just can't cover control families it has no
> signature for. `WDLayoutAnalyzeBMP` has no such limitation.

---

## Quick start A — screenshot layout detection (recommended)

1. Copy `wd_style32.dll` / `wd_style64.dll` (see [`prebuilt/`](prebuilt/)) next to your EXE.
2. Grab the control's image, save it as a BMP buffer, call the DLL, unpack the result.
   WinDev knows whether the control has a caption text and an image (both are
   properties) — pass those two facts as hints so the detector knows how many elements
   to look for.

```wlanguage
sDLL is string = In64bitMode() ? "wd_style64.dll" ELSE "wd_style32.dll"

imgBtn   is Image  = ScreenCaptureControl(sControlName)   // or your own capture
bufBMP   is Buffer = dSaveImageBMP(imgBtn)                 // 24-bit BMP in memory
sCapAnsi is ANSI string = Control..Caption                 // caption TEXT (not 0/1!)
nImg     is int    = (Control..Image <> "") ? 1 ELSE 0    // image hint

nP is int = CallDLL32(sDLL, "WDLayoutAnalyzeBMP2", &bufBMP, Length(bufBMP), &sCapAnsi, nImg)
// nP is a packed bitfield, see "Layout return value" below.
```

**Why pass the caption text instead of a 0/1 hint (v3)?** A control's caption property
can contain text while its *display* is switched off in the style (icon-only buttons).
v3 therefore **detects** whether a caption is actually rendered — the string only tells
the detector how many glyphs to expect, which also lets it split off icons glued
directly to the text (juxtaposed). Validated on 207 labeled buttons across three very
different themes (Eleven light, Cobalt pastel pills, Ankaa dark): caption presence
**206/207**, icon side (left/center/right) ≈ 93 %.

A ready-made `DetectButtonLayout()` procedure that unpacks `nP` into the five fields
`display_caption`, `vertical/horizontal_display_pos`, `has_image`,
`vertical/horizontal_image_pos` is in
[`examples/WinDev_Layout_Detection.wl.txt`](examples/WinDev_Layout_Detection.wl.txt).

## Quick start B — caption position from the blob (legacy)

```wlanguage
bufStyle is Buffer = {sControlPath, indControl}..Style
IF Length(bufStyle) < 12 THEN RESULT -1

sDLL is string = In64bitMode() ? "wd_style64.dll" ELSE "wd_style32.dll"
nPos is int = CallDLL32(sDLL, "WDStyleCaptionPosition", &bufStyle, Length(bufStyle))

// 13/14 = inherited from the style sheet: resolve once per sheet via the hash
IF nPos = 13 OR nPos = 14 THEN
	nHash is int = CallDLL32(sDLL, "WDStyleBaseHash", &bufStyle, Length(bufStyle))
	SWITCH nHash
		CASE 1687076956: nPos = 4   // example: this sheet's default = Centered
	END
END
RESULT nPos
```

---

## Exported functions

All functions are `extern "C"`, undecorated names, callable via `CallDLL32()` or `API()`.

### Layout from screenshot (approach A)

| Function | Description |
|---|---|
| `WDLayoutAnalyzeBMP2(ptr, len, caption, imageHint) -> int` | **v3, recommended.** Analyze a BMP crop (24/32-bit, BI_RGB). `caption` = the control's caption **text** (null-terminated ANSI; NULL/empty = property empty). Whether the caption is *displayed* is detected. Returns a packed bitfield (below) |
| `WDLayoutAnalyzeRaw2(ptr, w, h, stride, fmt, caption, imageHint) -> int` | v3 from raw pixels. `fmt`: 0=RGB, 1=BGR, 2=BGRA, 3=RGBA; `stride=0` = tightly packed |
| `WDLayoutBgColor(ptr, len) -> int` | Button fill color as `0xRRGGBB` (for 1:1 rebuilds) |
| `WDLayoutIsDark(ptr, len) -> int` | 1 = dark button background, 0 = light |
| `WDLayoutFgColor(ptr, len, caption, imageHint) -> int` | Detected caption text color as `0xRRGGBB`; `-3` = no caption rendered |
| `WDLayoutAnalyzeBMP(ptr, len, captionHint, imageHint) -> int` | v2 (kept for compatibility): 0/1 hints are *trusted* — an icon-only button with non-empty caption property is misreported. Prefer v3 |
| `WDLayoutAnalyzeRaw(ptr, w, h, stride, fmt, captionHint, imageHint) -> int` | v2 raw-pixel variant |

Three v3 details worth knowing. First, `imageHint` is a hint about the *image
property*, not a hard ceiling on whether an icon can be found — many buttons ("New",
"Print", "Delete", nav arrows) draw their icon from the **style sheet** with the image
property left empty. The detector always looks for an icon once a caption chain is
found; `imageHint=0` only means it applies two extra guards before accepting one (the
icon must sit with visible separation from the caption, or there must be more
components than the caption has glyphs) — otherwise a stray text fragment could be
misread as an icon. Second, if no caption chain is found at all *and* the image
property is empty, everything rendered is treated as the caption — this is what makes
one/two-glyph captions work (record-navigation buttons captioned `<`, `>>`, …). Third,
an icon drawn in a color that is *also* a dominant background color (white icon, pale
fill, white window corners) is handled by an island-segmentation fallback (regions of a
dominant color that do not touch the image border are ink); only an icon glued directly
to the text with *no* visible gap while the image property is empty, or glyphs that
visually merge into a border-touching glare band, remain a real edge case. Note also
that the DLL reports what is **rendered** — the same designer setting can render
differently per theme (observed: "left caption + left image" renders icon-left in
Eleven but icon-right in Ankaa).

### Blob decode & fingerprints (approach B + helpers)

| Function | Description |
|---|---|
| `WDStyleCaptionPosition(ptr, len) -> int` | Caption position of a *known* button style sheet (codes below); `14` if inherited/unknown |
| `WDStyleBaseHash(ptr, len) -> int` | Positive 31-bit hash of the style-sheet base region. Same sheet ⇒ same hash. Resolves inherited values (13/14) via a small lookup |
| `WDStyleUncompress(ptr, len, out, outCap) -> int` | Decompresses the **complete** style stream. Returns total length; call with `out = NULL` to query the size first |
| `WDStyleUncompressedSize(ptr, len) -> int` | Uncompressed size declared in the header |
| `WDStyleHasColTitleImage(ptr, len) -> int` | Table styles: 1 if a column-title background image is set, 0 if not |
| `WDStyleColTitleImageId(ptr, len) -> int` | Table styles: stable fingerprint of the column-title image (0 = none). Map fingerprint → filename via lookup |

### Button state-image steps

WinDev button background images pack **S states** (side by side) × **P steps**
(animation phases, stacked vertically). The number of steps is not exposed by
WLanguage; the number of states is (`..NumberOfStates`). These helpers detect and
strip surplus steps so you keep one step per state.

| Function | Description |
|---|---|
| `WDImageStepCount(bmpPtr, len, numStates) -> int` | Detected step count of a BMP state-image (1 or 6). Negative = error |
| `WDImageReduceSteps(bmpPtr, len, numStates, out, outCap) -> int` | If multi-step, returns a BMP cropped to the **top step** (full width, all states); a 1-step image is returned unchanged. Call with `out = NULL` to query the size first |

Detection is size-robust: a 1-step and a 6-step image can share the same pixel
dimensions, so it uses the **cell ratio** `width / (numStates × height)` — `≥ 0.7`
means one step (wide cells), below means six steps (cells 6× too flat). Verified on
66 diverse real state-images.

Negative returns are errors: `-1` bad arguments, `-2` bad/unrecognized header (or BMP),
`-3` decode error.

### Layout return value (packed bitfield)

`WDLayout*` returns a non-negative `int`. Unpack with `BinaryAND` / `BinaryShiftRight`:

| Bits | Field | Values |
|---|---|---|
| 0–1 | `horizontal_display_pos` | 0=none, 1=left, 2=center, 3=right |
| 2–3 | `vertical_display_pos` | 0=none, 1=top, 2=center, 3=bottom |
| 4 | `display_caption` | 0 / 1 |
| 5–6 | `horizontal_image_pos` | 0=none, 1=left, 2=center, 3=right |
| 7–8 | `vertical_image_pos` | 0=none, 1=top, 2=center, 3=bottom |
| 9 | `has_image` | 0 / 1 |

Verified (v3) against **207 labeled test buttons** across three very different themes
(Eleven light / Cobalt pastel pills / Ankaa dark bordered) — the whole corpus ships in
[`samples_layout/`](samples_layout/) and runs as a `cargo test` that compares the Rust
port field-by-field against the reference prototype: caption presence 206/207, icon
side ≈ 93 % — plus the earlier mixed set of 48 real production buttons. The horizontal
axis (left/center/right — WinDev's main distinction) is reliable throughout; the
vertical position of full-height icons and tiny glued (juxtaposed) icons can still be
a touch imprecise.

### Return codes of `WDStyleCaptionPosition` (approach B)

| Code | Meaning |
|---|---|
| 0 | Unknown (parsed, value not recognized) |
| 1 | No caption |
| 2 | Top |
| 3 | Bottom |
| 4 | Centered |
| 5 | Left |
| 6 | Right |
| 7 | Centered + juxtaposed image |
| 8 | Centered + image on the left |
| 9 | Centered + image on the right |
| 10 | Left + image on the left |
| 11 | Right + image on the right |
| 12 | Free positioning |
| 13 | Position inherited (caption visible, position from the style sheet → `WDStyleBaseHash`) |
| 14 | Fully inherited / not recognized → use approach A, or `WDStyleBaseHash` |

**Overrides vs. inheritance:** WinDev stores a property in the control's style blob
**only when it was explicitly changed away from the style sheet**. Untouched properties
are inherited at render time and are *not in the blob at all* — that is why 13/14 exist,
and why approach A (which reads the *rendered* result) is more general.

---

## The reverse-engineered `..Style` format

```
Offset 0      u8       compression type (0x04 = LZHUF)
Offset 1..5   u32 LE   compressed size   (= buffer length - 11)
Offset 5..9   u32 LE   uncompressed size of the FIRST logical block
Offset 9..11  u16      0x0000
Offset 11..   LZHUF bitstream (Okumura/Yoshizaki 1989 "lzhuf.c",
              adaptive Huffman + LZSS, 4096-byte ring buffer, MSB-first).
              PC SOFT variant: ring buffer initialized with 0x00
              (the original uses 0x20!).
```

Findings that matter if you want to dig deeper:

* The LZHUF stream **continues beyond the declared uncompressed size** — sub-style
  blocks follow (per-state visuals, image arrangement, coordinates).
  `WDStyleUncompress` gives you everything.
* Override values are stored as byte-aligned property tokens, but the **values
  themselves are bit-packed / scrambled** — not plain enums, RGB colors, or ASCII
  strings. A one-bit logical change can shift everything after it, so colors and image
  paths are currently only accessible as **stable fingerprints** (lookup), not decoded
  plaintext. Evidence in
  [`docs/BEFUND_Titel_Hintergrundfarbe.md`](docs/BEFUND_Titel_Hintergrundfarbe.md) (German).
* This bit-packed scramble is exactly why the screenshot approach (A) is the pragmatic
  winner for layout: it never touches the scrambled values.
* Style blobs are **deterministic** and `..Style` is writable, so
  `NewControl..Style = OldControl..Style` clones the full look without any decoding.

---

## Building from source

Requires Rust (stable, MSVC toolchain):

```bash
cargo build --release --target x86_64-pc-windows-msvc
cargo build --release --target i686-pc-windows-msvc
cargo test
```

Static CRT is configured in [`.cargo/config.toml`](.cargo/config.toml), so the DLLs
only import `KERNEL32`/`ntdll` — no `vcruntime140.dll` needed on the target machine
(the dynamic CRT caused `LoadLibrary` error 126 where the redistributable was missing).

## Repository layout

```
src/lib.rs      LZHUF decoder + blob signatures + fingerprints + FFI (approach B)
src/layout.rs   BMP parser + connected-components layout detector (approach A)
examples/       WLanguage integration (German): DLL calls + a pure-WL LZHUF fallback
docs/           research notes on the scrambled value encoding (German)
prebuilt/       ready-to-use wd_style32.dll / wd_style64.dll
```

## Status & roadmap

**Working and field-tested:**
* Caption/icon layout from a screenshot (approach A) — robust across control types,
  style sheets, themes and WinDev versions. **Recommended.**
* Caption position from the blob (approach B) for known button style sheets;
  style-sheet hashing; full-stream decompression; table column-title image
  detection/fingerprinting.

**Open / nice-to-have:**
* A: sharpen vertical icon precision and juxtaposed icon+text splitting.
* B: reverse the **value scramble** (bit-level variable-length code) — one routine
  apparently covers caption-position values, colors and image paths. Most promising
  route: disassembling the serialization in the WinDev runtime DLLs.

## Credits

Reverse-engineered and implemented by [Harveyhase68](https://github.com/Harveyhase68)
together with Claude (Anthropic). LZHUF by Haruyasu Yoshizaki and Haruhiko Okumura (1989).

WinDev and WLanguage are trademarks of PC SOFT. This project is not affiliated with
PC SOFT; it only reads data structures of your own applications at runtime.

## License

[MIT](LICENSE)
