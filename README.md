# wd_style — Read WinDev's undocumented `..Style` buffer

A tiny, dependency-free Rust DLL that **decompresses and parses the binary `..Style`
property of WinDev controls** — so your WLanguage code can finally answer questions
the official API cannot, such as:

> *"Which caption position (Top / Bottom / Left / Right / Centered / No caption /
> Free positioning …) does this button have at runtime?"*

There is **no documented WLanguage property** for the caption position of a button
(checked against the official PC SOFT documentation). The information only exists
inside the binary style blob. This project reverse-engineered that blob.

Works with 32-bit and 64-bit WinDev applications. No VC++ Redistributable required
(static CRT). Tested against WinDev 2024/2025 projects with 700+ real-world buttons.

---

## Quick start (WinDev / WLanguage)

1. Copy `wd_style32.dll` and `wd_style64.dll` (see [`prebuilt/`](prebuilt/) or build
   from source) next to your EXE.
2. Call it like any classic DLL:

```wlanguage
PROCEDURE ButtonCaptionPosition(sControlPath is string): int

bufStyle is Buffer = {sControlPath, indControl}..Style
IF Length(bufStyle) < 12 THEN RESULT -1

sDLL is string = In64bitMode() ? "wd_style64.dll" ELSE "wd_style32.dll"
nPos is int = CallDLL32(sDLL, "WDStyleCaptionPosition", &bufStyle, Length(bufStyle))

// Inherited values (13/14): the position is not stored in the control's blob
// but comes from its style sheet. Map it once per style sheet via the hash:
IF nPos = 13 OR nPos = 14 THEN
	nHash is int = CallDLL32(sDLL, "WDStyleBaseHash", &bufStyle, Length(bufStyle))
	SWITCH nHash
		CASE 1687076956: nPos = 4   // example: my project's default sheet = Centered
		// ...one CASE per style sheet in your project, or simply: nPos = 4
	END
END
RESULT nPos
```

Complete, commented WLanguage examples (German) are in [`examples/`](examples/),
including a **pure-WLanguage LZHUF decoder** if you cannot ship a DLL.

---

## Exported functions

All functions are `extern "C"`, undecorated names, callable via `CallDLL32()` or `API()`.

| Function | Description |
|---|---|
| `WDStyleCaptionPosition(ptr, len) -> int` | Caption position of a Button style (codes below) |
| `WDStyleBaseHash(ptr, len) -> int` | Positive 31-bit hash of the style-sheet base region. Same style sheet ⇒ same hash, independent of the caption position. Use it to resolve inherited values (13/14) with a small lookup |
| `WDStyleUncompress(ptr, len, out, outCap) -> int` | Decompresses the **complete** style stream (including sub-style blocks beyond the declared size). Returns total length; call with `out = NULL` to query the size first |
| `WDStyleUncompressedSize(ptr, len) -> int` | Uncompressed size declared in the header (e.g. 1498 for a plain button, 2114 with free positioning) |
| `WDStyleHasColTitleImage(ptr, len) -> int` | Table styles: 1 if a column-title background image is set, 0 if not |
| `WDStyleColTitleImageId(ptr, len) -> int` | Table styles: stable fingerprint of the column-title image (0 = none). Map fingerprint → filename via lookup |

`ptr, len` is always the raw `..Style` buffer and its length. Negative return values
are errors: `-1` bad arguments, `-2` not a recognized style header, `-3` decode error.

### Return codes of `WDStyleCaptionPosition`

| Code | Meaning | |
|---|---|---|
| 0 | Unknown | style parsed, but value not recognized — please open an issue with the blob! |
| 1 | No caption | explicit override |
| 2 | Top | explicit override |
| 3 | Bottom | explicit override |
| 4 | Centered | explicit override |
| 5 | Left | explicit override |
| 6 | Right | explicit override |
| 7 | Centered + juxtaposed image | explicit override |
| 8 | Centered + image on the left | explicit override |
| 9 | Centered + image on the right | explicit override |
| 10 | Left + image on the left | explicit override |
| 11 | Right + image on the right | explicit override |
| 12 | Free positioning | |
| 13 | Position inherited | caption explicitly visible, position comes from the style sheet → resolve via `WDStyleBaseHash` |
| 14 | Fully inherited | nothing overridden, everything comes from the style sheet → resolve via `WDStyleBaseHash` |

**Overrides vs. inheritance — important:** WinDev stores a property in the control's
style blob **only when it was explicitly changed away from the style sheet** in the
editor. Untouched properties are inherited at render time and are *not present in the
blob at all*. That is why 13/14 exist: the answer genuinely isn't in the buffer.
In a scan of 669 production buttons this two-stage approach (codes 1–12 direct,
13/14 via one hash lookup per style sheet) resolved **100 %** of the controls.

---

## The reverse-engineered format

The `..Style` buffer (read/write via WLanguage since v20) has this layout:

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
  blocks follow (per-state visuals, image arrangement, free-positioning coordinates).
  `WDStyleUncompress` gives you everything.
* The decompressed content is a property stream: byte-aligned property tokens for
  overrides (e.g. caption visibility `34 B4 81 77 F4 C4` + value, caption position
  `6D EF 5F A4 DD 79 9F D0 18 77` + value), but the **values themselves are
  bit-packed / scrambled** — they are not plain enums, RGB colors, or ASCII strings.
* Because of the bit-packing, a one-bit logical change can shift everything after it.
  Colors and image paths are therefore currently only accessible as **stable
  fingerprints** (lookup), not as decoded plaintext. Details and evidence in
  [`docs/BEFUND_Titel_Hintergrundfarbe.md`](docs/BEFUND_Titel_Hintergrundfarbe.md)
  (German).
* Style blobs are **deterministic**: two controls with identical style settings have
  byte-identical `..Style` buffers — and `..Style` is writable, so
  `NewControl..Style = OldControl..Style` clones the full look (including image
  references) without any decoding.

---

## Building from source

Requires Rust (stable, MSVC toolchain):

```bash
cargo build --release --target x86_64-pc-windows-msvc
cargo build --release --target i686-pc-windows-msvc
```

The DLLs land in `target/<target>/release/wd_style.dll`. Static CRT is configured in
[`.cargo/config.toml`](.cargo/config.toml), so the DLLs only import `KERNEL32`/`ntdll`
— no `vcruntime140.dll` needed on the target machine (this matters: the dynamic CRT
caused `LoadLibrary` error 126 on machines where the redistributable was missing).

Run the test suite (83 sample blobs included in [`samples/`](samples/)):

```bash
cargo test --release
```

## Repository layout

```
src/lib.rs      DLL source: LZHUF decoder + signature detection + FFI exports
samples/        83 real ..Style blobs (12 caption-position variants × multiple
                buttons/windows, table styles with color/image variations)
examples/       WLanguage integration (German comments): DLL calls + a pure-WL
                LZHUF decoder as fallback
docs/           research notes on the scrambled value encoding (German)
prebuilt/       ready-to-use wd_style32.dll / wd_style64.dll
```

## Status & roadmap

Working and field-tested: caption position (buttons), style-sheet hashing,
full-stream decompression, table column-title image detection/fingerprinting.

Open research (contributions welcome!):

* Reverse the **value scramble** (bit-level variable-length code) — one routine
  apparently covers caption-position values, colors and image paths. The most
  promising route is disassembling the serialization in the WinDev runtime DLLs.
* More control types and properties — the token-search approach generalizes:
  export two `..Style` blobs differing in exactly one property, diff the
  decompressed streams, add the signature.

## Credits

Reverse-engineered and implemented by [Harveyhase68](https://github.com/Harveyhase68)
together with Claude (Anthropic). The LZHUF algorithm is by Haruyasu Yoshizaki and
Haruhiko Okumura (1989).

WinDev and WLanguage are trademarks of PC SOFT. This project is not affiliated with
PC SOFT; it only reads data structures of your own applications at runtime.

## License

[AGPL-3.0](LICENSE)
