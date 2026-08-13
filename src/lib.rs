//! wd_style.dll — WinDev ..Style-Buffer Decoder
//! =============================================
//! Reverse-engineered Format des WLanguage-Property-Buffers `..Style`:
//!
//!   Offset 0     : u8      Kompressionstyp (0x04 = LZHUF)
//!   Offset 1..5  : u32 LE  komprimierte Groesse (= Puffergroesse - 11)
//!   Offset 5..9  : u32 LE  unkomprimierte Groesse des ERSTEN Blocks
//!                          (Button normal: 1498, Free positioning: 2114)
//!   Offset 9..11 : u16     0x0000
//!   Offset 11..  : LZHUF-Bitstream (Okumura/Yoshizaki 1989, MSB-first,
//!                  Ringpuffer 4096 Bytes mit 0x00 initialisiert).
//!                  Der Stream enthaelt ueber die deklarierte Groesse hinaus
//!                  weitere Sub-Style-Bloecke (Bild-Anordnung usw.).
//!
//! Export-Funktionen (extern "C", WinDev-kompatibel via CallDLL32/API()):
//!   WDStyleCaptionPosition(ptr, len)            -> i32 (CaptionPosition enum)
//!   WDStyleUncompressedSize(ptr, len)           -> i32 (deklarierte Blockgroesse)
//!   WDStyleUncompress(ptr, len, out, out_cap)   -> i32 (voller Stream; siehe Doku)

const RING: usize = 4096;
const F: usize = 60;
const THRESHOLD: usize = 2;
const N_CHAR: usize = 314; // 256 Literale + 58 Matchlaengen
const T: usize = N_CHAR * 2 - 1; // 627
const ROOT: usize = T - 1; // 626
const MAX_FREQ: u32 = 0x8000;
const MAX_OUTPUT: usize = 1 << 20; // 1 MiB Sicherheitslimit

// ---------------------------------------------------------------------------
// Enum wie mit dem Anwender vereinbart (entspricht den 12 Editor-Kacheln)
// ---------------------------------------------------------------------------
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptionPosition {
    Unknown = 0,
    NoCaption = 1,
    Top = 2,
    Bottom = 3,
    Center = 4,
    Left = 5,
    Right = 6,
    CenterJuxtaposedImage = 7,
    CenterLeftImage = 8,
    CenterRightImage = 9,
    LeftLeftImage = 10,
    RightRightImage = 11,
    FreePositioning = 12,
    /// Caption per Override sichtbar, Position NICHT ueberschrieben ->
    /// Position kommt aus dem Gabarit (Style-Sheet). Mit WDStyleBaseHash
    /// und einer kleinen Lookup-Tabelle aufloesen.
    PositionInherited = 13,
    /// Weder Caption noch Position ueberschrieben -> alles kommt aus dem
    /// Gabarit. Mit WDStyleBaseHash aufloesen.
    Inherited = 14,
}

/// Fehlercodes (negativ, damit vom Enum unterscheidbar)
pub const ERR_BAD_ARGS: i32 = -1; // Nullpointer / zu kurz
pub const ERR_BAD_HEADER: i32 = -2; // kein 0x04-LZHUF-Header
pub const ERR_DECODE: i32 = -3; // Dekompression fehlgeschlagen

// --------------------------- Bit-Reader ------------------------------------
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    bit: u8,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0, bit: 0 }
    }
    #[inline]
    fn exhausted(&self) -> bool {
        self.pos >= self.data.len()
    }
    #[inline]
    fn get1(&mut self) -> u32 {
        let b = if self.pos < self.data.len() {
            (self.data[self.pos] >> (7 - self.bit)) & 1
        } else {
            0
        };
        self.bit += 1;
        if self.bit == 8 {
            self.bit = 0;
            self.pos += 1;
        }
        b as u32
    }
    #[inline]
    fn get(&mut self, n: u32) -> u32 {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.get1();
        }
        v
    }
}

// --------------------- Adaptiver Huffman-Baum (lzhuf.c) --------------------
struct Huff {
    freq: [u32; T + 1],
    prnt: [usize; T + N_CHAR],
    son: [usize; T],
}

impl Huff {
    fn new() -> Self {
        let mut h = Huff {
            freq: [0; T + 1],
            prnt: [0; T + N_CHAR],
            son: [0; T],
        };
        for i in 0..N_CHAR {
            h.freq[i] = 1;
            h.son[i] = i + T;
            h.prnt[i + T] = i;
        }
        let (mut i, mut j) = (0usize, N_CHAR);
        while j <= ROOT {
            h.freq[j] = h.freq[i] + h.freq[i + 1];
            h.son[j] = i;
            h.prnt[i] = j;
            h.prnt[i + 1] = j;
            i += 2;
            j += 1;
        }
        h.freq[T] = 0xFFFF;
        h.prnt[ROOT] = 0;
        h
    }

    /// Baum-Rekonstruktion, wenn die Wurzelfrequenz MAX_FREQ erreicht
    /// (bei Style-Puffern < 32 KiB nie noetig, aber der Vollstaendigkeit halber).
    fn reconst(&mut self) {
        let mut j = 0usize;
        for i in 0..T {
            if self.son[i] >= T {
                self.freq[j] = (self.freq[i] + 1) / 2;
                self.son[j] = self.son[i];
                j += 1;
            }
        }
        let (mut i, mut j) = (0usize, N_CHAR);
        while j < T {
            let k = i + 1;
            let f = self.freq[i] + self.freq[k];
            self.freq[j] = f;
            let mut k = j - 1;
            while f < self.freq[k] {
                k -= 1;
            }
            let k = k + 1;
            for m in ((k + 1)..=j).rev() {
                self.freq[m] = self.freq[m - 1];
                self.son[m] = self.son[m - 1];
            }
            self.freq[k] = f;
            self.son[k] = i;
            i += 2;
            j += 1;
        }
        for i in 0..T {
            let k = self.son[i];
            self.prnt[k] = i;
            if k < T {
                self.prnt[k + 1] = i;
            }
        }
    }

    fn update(&mut self, sym: usize) {
        if self.freq[ROOT] == MAX_FREQ {
            self.reconst();
        }
        let mut c = self.prnt[sym + T];
        loop {
            self.freq[c] += 1;
            let k = self.freq[c];
            let mut l = c + 1;
            if k > self.freq[l] {
                while k > self.freq[l + 1] {
                    l += 1;
                }
                self.freq[c] = self.freq[l];
                self.freq[l] = k;
                let i = self.son[c];
                self.prnt[i] = l;
                if i < T {
                    self.prnt[i + 1] = l;
                }
                let j = self.son[l];
                self.son[l] = i;
                self.prnt[j] = c;
                if j < T {
                    self.prnt[j + 1] = c;
                }
                self.son[c] = j;
                c = l;
            }
            c = self.prnt[c];
            if c == 0 {
                break;
            }
        }
    }

    fn decode_char(&mut self, bits: &mut Bits) -> usize {
        let mut c = self.son[ROOT];
        while c < T {
            c += bits.get1() as usize;
            c = self.son[c];
        }
        c -= T;
        self.update(c);
        c
    }
}

// ------------- Positions-Dekodiertabellen (statisch, kanonisch) ------------
// Codelaengen der oberen 6 Positionsbits: 1x3, 3x4, 8x5, 12x6, 24x7, 16x8
fn build_pos_tables() -> ([u8; 256], [u8; 256]) {
    let mut p_len = [0u8; 64];
    for v in 0..64 {
        p_len[v] = match v {
            0 => 3,
            1..=3 => 4,
            4..=11 => 5,
            12..=23 => 6,
            24..=47 => 7,
            _ => 8,
        };
    }
    let mut d_code = [0u8; 256];
    let mut d_len = [0u8; 256];
    let mut code: usize = 0;
    for v in 0..64 {
        let l = p_len[v] as usize;
        let span = 1usize << (8 - l);
        let from = code * span;
        for i in from..from + span {
            d_code[i] = v as u8;
            d_len[i] = l as u8;
        }
        code += 1;
        if v < 63 && p_len[v + 1] > p_len[v] {
            code <<= (p_len[v + 1] - p_len[v]) as usize;
        }
    }
    (d_code, d_len)
}

fn decode_position(bits: &mut Bits, d_code: &[u8; 256], d_len: &[u8; 256]) -> usize {
    let mut i = bits.get(8) as usize;
    let c = (d_code[i] as usize) << 6;
    for _ in 0..(d_len[i] - 2) {
        i = (i << 1) + bits.get1() as usize;
    }
    c | (i & 0x3F)
}

// ------------------------------ Decoder ------------------------------------

/// Header pruefen; liefert (deklarierte unkomprimierte Groesse, Payload).
fn parse_header(data: &[u8]) -> Result<(usize, &[u8]), i32> {
    if data.len() < 12 {
        return Err(ERR_BAD_HEADER);
    }
    if data[0] != 4 {
        return Err(ERR_BAD_HEADER);
    }
    let orig = u32::from_le_bytes([data[5], data[6], data[7], data[8]]) as usize;
    if orig == 0 || orig > MAX_OUTPUT {
        return Err(ERR_BAD_HEADER);
    }
    Ok((orig, &data[11..]))
}

/// Dekomprimiert den KOMPLETTEN LZHUF-Stream (auch ueber die im Header
/// deklarierte Groesse hinaus, denn dahinter folgen die Sub-Style-Bloecke).
fn lzhuf_decode_all(payload: &[u8]) -> Vec<u8> {
    let (d_code, d_len) = build_pos_tables();
    let mut bits = Bits::new(payload);
    let mut huf = Huff::new();
    let mut ring = [0u8; RING];
    let mut r = RING - F;
    let mut out: Vec<u8> = Vec::with_capacity(4096);
    while !bits.exhausted() && out.len() < MAX_OUTPUT {
        let c = huf.decode_char(&mut bits);
        if c < 256 {
            let ch = c as u8;
            out.push(ch);
            ring[r] = ch;
            r = (r + 1) & (RING - 1);
        } else {
            let pos = decode_position(&mut bits, &d_code, &d_len);
            let idx = (r + RING - pos - 1) & (RING - 1);
            let len = c - 255 + THRESHOLD;
            for k in 0..len {
                let ch = ring[(idx + k) & (RING - 1)];
                out.push(ch);
                ring[r] = ch;
                r = (r + 1) & (RING - 1);
            }
        }
    }
    out
}

// --------------------------- Signaturen ------------------------------------
// Verifiziert an 41 Samples (4 Buttons, 2 Fenster, 12 Editor-Optionen).
const TOK_CAPTION: &[u8] = &[0x34, 0xB4, 0x81, 0x77, 0xF4, 0xC4];
const VAL_NO_CAPTION: &[u8] = &[0x24, 0x00, 0x00, 0x00];
const VAL_CAPTION_ON: &[u8] = &[0x9A, 0xA9, 0xC4, 0x22];
const TOK_POSITION: &[u8] = &[0x6D, 0xEF, 0x5F, 0xA4, 0xDD, 0x79, 0x9F, 0xD0, 0x18, 0x77];
const POSVAL_TOP: &[u8] = &[0x8F, 0x97, 0x5F, 0xD9];
const POSVAL_BOTTOM: &[u8] = &[0x8F, 0xC8, 0xE6, 0x00];
const POSVAL_CENTER: &[u8] = &[0x8F, 0xC8, 0x49, 0x00];
const POSVAL_RIGHT: &[u8] = &[0x8F, 0xCD, 0xC8, 0x00];
const POSVAL_LEFT: &[u8] = &[0x50, 0x00, 0x00, 0x00];
// Bild-Anordnung (tiefer im Stream; bisher nur an einem Fenster verifiziert):
const SIG_JUXTAPOSED: &[u8] = &[0x9A, 0xA9, 0x12, 0xB4, 0xF2, 0xF3];
const SIG_IMG_LEFT: &[u8] = &[0x50, 0xA5, 0x3E, 0xB6];
const SIG_IMG_RIGHT: &[u8] = &[0x50, 0xA5, 0x3E, 0x2C];
const SIG_LEFT_LEFT: &[u8] = &[0x65, 0xFD, 0x2C, 0xD8, 0xC4, 0xED, 0x7E, 0xDD];
const SIG_RIGHT_RIGHT: &[u8] = &[0xD8, 0xEC, 0x70, 0x1C, 0xBE, 0xFA, 0xEB, 0xD8];
// Free positioning: deklarierte Blockgroesse waechst um ~616 Bytes
// (normale Buttons im Feld: 1278..1508, free positioning: 2114)
const FREE_THRESHOLD: usize = 1600;

// ---------------------------------------------------------------------------
// Spaltentitel-/Border-Hintergrundbild (Schritt 2, Stand 2026-08-09)
// ---------------------------------------------------------------------------
// Befund an WIN_TEST3.TABLE_TEST3_* Samples:
//   * Ist ein Bild gesetzt, taucht im entpackten Stream der Anker
//     9A D4 0B 7E 4D auf (bei "kein Bild" fehlt er komplett) -> robustes
//     Ja/Nein-Kriterium.
//   * Der Bildname/-pfad selbst ist NICHT Klartext, sondern ein
//     laengenvariabler, positionsabhaengiger Zeichen-Scramble
//     (weder XOR noch Standard-Hash). Verifiziert mit bild1/2/3.png:
//     eine 1-Zeichen-Aenderung des Dateinamens aendert genau EIN Byte.
//   * IMG_WINDOW ab dem Anker liefert einen Fingerprint, der verschiedene
//     Bilder unterscheidet. ACHTUNG: das Fenster ueberlappt bei gesetztem
//     Bild ab ~Offset 4075 die (ebenfalls verwuerfelte) Farb-Region, d.h.
//     der Fingerprint ist farb-konfundiert -> nur zuverlaessig, wenn die
//     Titelfarbe konstant gehalten wird. Die saubere, farb-unabhaengige
//     Loesung ist das Reversen des Scrambles (Weg B) -> dann echter Name.
const IMG_ANCHOR: &[u8] = &[0x9A, 0xD4, 0x0B, 0x7E, 0x4D];
const IMG_WINDOW: usize = 256;

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Kernlogik: Style-Puffer -> CaptionPosition
pub fn caption_position(style: &[u8]) -> i32 {
    let (orig, payload) = match parse_header(style) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let raw = lzhuf_decode_all(payload);
    if raw.len() < orig {
        return ERR_DECODE;
    }

    // 1) Caption-Override auswerten (die Token sind OVERRIDES: sie existieren
    //    nur, wenn die Eigenschaft vom Gabarit abweichend gesetzt wurde!)
    //    None = nicht ueberschrieben -> vom Gabarit geerbt.
    let cap_override = match find(&raw, TOK_CAPTION) {
        Some(i) if raw.len() >= i + 10 => Some(&raw[i + 6..i + 10]),
        _ => None,
    };
    if let Some(cap) = cap_override {
        if cap == VAL_NO_CAPTION {
            return CaptionPosition::NoCaption as i32;
        }
        if cap != VAL_CAPTION_ON {
            return CaptionPosition::Unknown as i32;
        }
    }

    // 2) Free positioning: eigener (groesserer) Hauptblock
    if orig > FREE_THRESHOLD {
        return CaptionPosition::FreePositioning as i32;
    }

    // 3) Text-Position (Override)
    let pos = match find(&raw, TOK_POSITION) {
        Some(i) if raw.len() >= i + 14 => &raw[i + 10..i + 14],
        _ => {
            // Kein Positions-Override -> Position kommt aus dem Gabarit.
            return if cap_override.is_some() {
                CaptionPosition::PositionInherited as i32
            } else {
                CaptionPosition::Inherited as i32
            };
        }
    };
    if pos == POSVAL_TOP {
        return CaptionPosition::Top as i32;
    }
    if pos == POSVAL_BOTTOM {
        return CaptionPosition::Bottom as i32;
    }
    if pos == POSVAL_CENTER {
        // 4a) Bild-Anordnung innerhalb "Centered"
        if find(&raw, SIG_JUXTAPOSED).is_some() {
            return CaptionPosition::CenterJuxtaposedImage as i32;
        }
        if find(&raw, SIG_IMG_LEFT).is_some() {
            return CaptionPosition::CenterLeftImage as i32;
        }
        if find(&raw, SIG_IMG_RIGHT).is_some() {
            return CaptionPosition::CenterRightImage as i32;
        }
        return CaptionPosition::Center as i32;
    }
    if pos == POSVAL_LEFT {
        if find(&raw, SIG_LEFT_LEFT).is_some() {
            return CaptionPosition::LeftLeftImage as i32;
        }
        return CaptionPosition::Left as i32;
    }
    if pos == POSVAL_RIGHT {
        if find(&raw, SIG_RIGHT_RIGHT).is_some() {
            return CaptionPosition::RightRightImage as i32;
        }
        return CaptionPosition::Right as i32;
    }
    CaptionPosition::Unknown as i32
}

// ------------------------------ FFI-Exporte --------------------------------

/// Liefert die CaptionPosition (Enum-Werte oben) fuer einen ..Style-Puffer.
/// WLanguage:  nPos = API("wd_style.dll","WDStyleCaptionPosition", bufStyle, Length(bufStyle))
#[no_mangle]
pub extern "C" fn WDStyleCaptionPosition(data: *const u8, len: i32) -> i32 {
    if data.is_null() || len < 12 {
        return ERR_BAD_ARGS;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    caption_position(slice)
}

/// FNV-1a-32 ueber die ersten 256 dekomprimierten Bytes, auf 31 Bit maskiert
/// (immer positiv, damit WLanguage-int-Vergleiche ohne Vorzeichen-Fallen
/// funktionieren). Die ersten ~680 Bytes enthalten die Gabarit-Referenz und
/// die Basis-Eigenschaften, aber KEINE Positions-Overrides -> der Hash ist
/// fuer alle Buttons desselben Gabarits gleich, unabhaengig von der
/// Caption-Position. Damit laesst sich fuer die Rueckgabewerte 13/14 eine
/// kleine Lookup-Tabelle "Hash -> Standard-Position des Gabarits" pflegen.
pub fn base_hash(style: &[u8]) -> i32 {
    let payload = match parse_header(style) {
        Ok((_, p)) => p,
        Err(e) => return e,
    };
    let raw = lzhuf_decode_all(payload);
    let n = raw.len().min(256);
    let mut h: u32 = 0x811C_9DC5;
    for &b in &raw[..n] {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    (h & 0x7FFF_FFFF) as i32
}

/// Gabarit-Hash fuer die Aufloesung vererbter Werte (siehe base_hash).
/// WLanguage:  nHash = CallDLL32("wd_style64.dll","WDStyleBaseHash", &bufStyle, Length(bufStyle))
#[no_mangle]
pub extern "C" fn WDStyleBaseHash(data: *const u8, len: i32) -> i32 {
    if data.is_null() || len < 12 {
        return ERR_BAD_ARGS;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    base_hash(slice)
}

/// Liefert die im Header deklarierte unkomprimierte Blockgroesse (z.B. 1498).
#[no_mangle]
pub extern "C" fn WDStyleUncompressedSize(data: *const u8, len: i32) -> i32 {
    if data.is_null() || len < 12 {
        return ERR_BAD_ARGS;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    match parse_header(slice) {
        Ok((orig, _)) => orig as i32,
        Err(e) => e,
    }
}

/// Dekomprimiert den KOMPLETTEN Stream in den Ausgabepuffer.
/// Rueckgabe: Gesamtlaenge des entpackten Streams (auch wenn out_cap kleiner
/// ist -- dann wird nur out_cap geschrieben; mit out=NULL/out_cap=0 laesst
/// sich die noetige Groesse abfragen). Negativ = Fehler.
/// WLanguage:
///   bufRaw is Buffer = RepeatString(Charact(0), 65536)
///   nLen is int = API("wd_style.dll","WDStyleUncompress", bufStyle, Length(bufStyle), bufRaw, Length(bufRaw))
///   bufRaw = bufRaw[[1 TO nLen]]
#[no_mangle]
pub extern "C" fn WDStyleUncompress(
    data: *const u8,
    len: i32,
    out: *mut u8,
    out_cap: i32,
) -> i32 {
    if data.is_null() || len < 12 {
        return ERR_BAD_ARGS;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    let (_, payload) = match parse_header(slice) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let raw = lzhuf_decode_all(payload);
    if !out.is_null() && out_cap > 0 {
        let n = raw.len().min(out_cap as usize);
        unsafe { std::ptr::copy_nonoverlapping(raw.as_ptr(), out, n) };
    }
    raw.len() as i32
}

// ------------------- Spaltentitel-Bild: Kernlogik --------------------------

/// FNV-1a-32 ueber ein Byte-Fenster, auf 31 Bit maskiert und nie 0
/// (0 ist fuer "kein Bild" reserviert).
fn fnv1a_window(buf: &[u8]) -> i32 {
    let mut h: u32 = 0x811C_9DC5;
    for &b in buf {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    let r = (h & 0x7FFF_FFFF) as i32;
    if r == 0 {
        1
    } else {
        r
    }
}

/// true, wenn ein Spaltentitel-/Border-Hintergrundbild gesetzt ist.
pub fn has_coltitle_image(style: &[u8]) -> bool {
    match parse_header(style) {
        Ok((_, payload)) => find(&lzhuf_decode_all(payload), IMG_ANCHOR).is_some(),
        Err(_) => false,
    }
}

/// Fingerprint des gesetzten Bildes fuer die Lookup-Tabelle (Weg A).
/// Rueckgabe:
///    0  kein Bild gesetzt (Anker fehlt)
///   <0  Fehler (parse_header)
///   >0  FNV-1a-Fingerprint des Bildfelds (nur bei konstanter Titelfarbe
///       zuverlaessig, siehe IMG_WINDOW-Doku).
pub fn coltitle_image_id(style: &[u8]) -> i32 {
    let payload = match parse_header(style) {
        Ok((_, p)) => p,
        Err(e) => return e,
    };
    let raw = lzhuf_decode_all(payload);
    match find(&raw, IMG_ANCHOR) {
        None => 0,
        Some(i) => {
            let end = (i + IMG_WINDOW).min(raw.len());
            fnv1a_window(&raw[i..end])
        }
    }
}

// ------------------- Spaltentitel-Bild: FFI-Exporte ------------------------

/// 1 = Bild gesetzt, 0 = kein Bild, negativ = Fehler.
/// WLanguage: n = API("wd_style64.dll","WDStyleHasColTitleImage", buf, Length(buf))
#[no_mangle]
pub extern "C" fn WDStyleHasColTitleImage(data: *const u8, len: i32) -> i32 {
    if data.is_null() || len < 12 {
        return ERR_BAD_ARGS;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    match parse_header(slice) {
        Ok(_) => {
            if has_coltitle_image(slice) {
                1
            } else {
                0
            }
        }
        Err(e) => e,
    }
}

/// Fingerprint des Spaltentitel-Bildes (siehe coltitle_image_id).
/// WLanguage:
///   nId is int = API("wd_style64.dll","WDStyleColTitleImageId", buf, Length(buf))
///   SWITCH nId
///     CASE 749989678:  sName = "bild1.png"
///     CASE 1083170495: sName = "bild2.png"
///     CASE 1058762980: sName = "bild3.png"
///     CASE 595445315:  sName = "Evolution2_Table_ColTitle.png"  // nur bei Farbe 000000/FFFFFF
///     OTHER CASE:      sName = ""  // unbekannt -> Sample exportieren und eintragen
///   END
#[no_mangle]
pub extern "C" fn WDStyleColTitleImageId(data: *const u8, len: i32) -> i32 {
    if data.is_null() || len < 12 {
        return ERR_BAD_ARGS;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    coltitle_image_id(slice)
}

// ------------------------------- Tests -------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Samples liegen im Repo unter samples/; Fallback: Projektordner darueber.
    fn sample(file: &str) -> Vec<u8> {
        let base = Path::new(env!("CARGO_MANIFEST_DIR"));
        let p = base.join("samples").join(file);
        let p = if p.exists() { p } else { base.join("..").join(file) };
        fs::read(&p).unwrap_or_else(|_| panic!("Sample fehlt: {}", file))
    }

    fn check(file: &str, expected: CaptionPosition) {
        let data = sample(file);
        let got = caption_position(&data);
        assert_eq!(
            got, expected as i32,
            "{}: erwartet {:?} ({}), bekommen {}",
            file, expected, expected as i32, got
        );
    }

    #[test]
    fn alle_samples() {
        use CaptionPosition::*;
        let cases: &[(&str, CaptionPosition)] = &[
            ("BTN_Test1_no_Caption.bin", NoCaption),
            ("BTN_Test2_no_Caption.bin", NoCaption),
            ("BTN_TEST3_no_Caption.bin", NoCaption),
            ("BTN_TEST4_no_Caption.bin", NoCaption),
            ("BTN_TEST1_top_Caption.bin", Top),
            ("BTN_TEST2_top_Caption.bin", Top),
            ("BTN_TEST3_top_Caption.bin", Top),
            ("BTN_TEST4_top_Caption.bin", Top),
            ("BTN_TEST1_bottom_Caption.bin", Bottom),
            ("BTN_TEST2_bottom_Caption.bin", Bottom),
            ("BTN_TEST3_bottom_Caption.bin", Bottom),
            ("BTN_TEST4_bottom_Caption.bin", Bottom),
            ("BTN_TEST1_left_Caption.bin", Left),
            ("BTN_TEST2_left_Caption.bin", Left),
            ("BTN_TEST3_left_Caption.bin", Left),
            ("BTN_TEST4_left_Caption.bin", Left),
            ("BTN_TEST1_right_Caption.bin", Right),
            ("BTN_TEST2_right_Caption.bin", Right),
            ("BTN_TEST3_right_Caption.bin", Right),
            ("BTN_TEST4_right_Caption.bin", Right),
            ("BTN_TEST1_centered_Caption.bin", Center),
            ("BTN_TEST2_centered_Caption.bin", Center),
            ("BTN_TEST3_centered_Caption.bin", Center),
            ("BTN_TEST4_centerd_Caption.bin", Center),
            (
                "BTN_TEST3_centered_and_juxtaposed_image_Caption.bin",
                CenterJuxtaposedImage,
            ),
            (
                "BTN_TEST4_centerd_and_juxtaposed_image_Caption.bin",
                CenterJuxtaposedImage,
            ),
            (
                "BTN_TEST3_centered_and_left_image_Caption.bin",
                CenterLeftImage,
            ),
            (
                "BTN_TEST4_centerd_and_left_image_Caption.bin",
                CenterLeftImage,
            ),
            (
                "BTN_TEST3_centered_and_right_image_Caption.bin",
                CenterRightImage,
            ),
            (
                "BTN_TEST4_centerd_and_right_image_Caption.bin",
                CenterRightImage,
            ),
            ("BTN_TEST3_left_and_left_image_Caption.bin", LeftLeftImage),
            ("BTN_TEST4_left_and_left_image_Caption.bin", LeftLeftImage),
            (
                "BTN_TEST3_right_and_right_image_Caption.bin",
                RightRightImage,
            ),
            (
                "BTN_TEST4_right_and_right_image_Caption.bin",
                RightRightImage,
            ),
            ("BTN_TEST3_free_pos_Caption.bin", FreePositioning),
            ("BTN_TEST4_free_pos_Caption.bin", FreePositioning),
            (
                "BTN_TEST3_free_pos_20.25_24.24_35.45_55_65_Caption.bin",
                FreePositioning,
            ),
            (
                "BTN_TEST4_free_pos_20.25_24.24_35.45_55_65_Caption.bin",
                FreePositioning,
            ),
            (
                "BTN_TEST3_free_pos_20.25_24.24_35.45_55_65_homogetic_centered_Caption.bin",
                FreePositioning,
            ),
            (
                "BTN_TEST4_free_pos_20.25_24.24_35.45_55_65_homogetic_centered_Caption.bin",
                FreePositioning,
            ),
        ];
        for (f, e) in cases {
            check(f, *e);
        }
    }

    #[test]
    fn ffi_smoke() {
        let data = sample("BTN_TEST3_top_Caption.bin");
        let r = WDStyleCaptionPosition(data.as_ptr(), data.len() as i32);
        assert_eq!(r, CaptionPosition::Top as i32);
        assert_eq!(
            WDStyleUncompressedSize(data.as_ptr(), data.len() as i32),
            1498
        );
        let needed = WDStyleUncompress(data.as_ptr(), data.len() as i32, std::ptr::null_mut(), 0);
        assert!(needed >= 1498, "needed={}", needed);
        let mut out = vec![0u8; needed as usize];
        let written =
            WDStyleUncompress(data.as_ptr(), data.len() as i32, out.as_mut_ptr(), needed);
        assert_eq!(written, needed);
    }

    /// Reale Projekt-Buttons (nur wenn das QNAP-Laufwerk verfuegbar ist)
    #[test]
    fn reale_buttons() {
        let d = Path::new(r"I:\My Projects\29\primaB2B\Exe\64-bit Windows executable");
        if !d.is_dir() {
            eprintln!("I:-Laufwerk nicht verfuegbar, Test uebersprungen");
            return;
        }
        let b3 = fs::read(d.join("Auswahl_Artikel_Select.Button3.bin")).unwrap();
        let b5 = fs::read(d.join("Auswahl_Artikel_Select.Button5.bin")).unwrap();
        let nc = fs::read(d.join("WIN_TEST2.BTN_NO_CAPTION.bin")).unwrap();
        // Button3: Caption-Override ON, Position vererbt
        assert_eq!(caption_position(&b3), CaptionPosition::PositionInherited as i32);
        // Button5: Override "keine Caption"
        assert_eq!(caption_position(&b5), CaptionPosition::NoCaption as i32);
        // BTN_NO_CAPTION: gar keine Overrides
        assert_eq!(caption_position(&nc), CaptionPosition::Inherited as i32);
        // Gleicher Gabarit -> gleicher Hash; anderer Gabarit -> anderer Hash
        assert_eq!(base_hash(&b3), base_hash(&b5));
        assert_ne!(base_hash(&b3), base_hash(&nc));
        assert!(base_hash(&b3) > 0);
    }

    #[test]
    fn coltitle_image() {
        let read = sample;
        // Ja/Nein-Gate
        assert!(has_coltitle_image(&read("WIN_TEST3.TABLE_TEST3_BILD1.bin")));
        assert!(!has_coltitle_image(&read("WIN_TEST3.TABLE_TEST3_FFFFFF.bin")));
        // Fingerprints (verschiedene Bilder -> verschiedene Ids)
        assert_eq!(coltitle_image_id(&read("WIN_TEST3.TABLE_TEST3_BILD1.bin")), 749989678);
        assert_eq!(coltitle_image_id(&read("WIN_TEST3.TABLE_TEST3_BILD2.bin")), 1083170495);
        assert_eq!(coltitle_image_id(&read("WIN_TEST3.TABLE_TEST3_BILD3.bin")), 1058762980);
        // kein Bild -> 0
        assert_eq!(coltitle_image_id(&read("WIN_TEST3.TABLE_TEST3_FFFFFF.bin")), 0);
        // Evolution-Bild (Farbe 000000/FFFFFF fallen zusammen)
        assert_eq!(
            coltitle_image_id(&read("WIN_TEST3.TABLE_TEST3_000000_IMAGE_GESETZT.bin")),
            595445315
        );
        assert_eq!(
            coltitle_image_id(&read("WIN_TEST3.TABLE_TEST3_FFFFFF_IMAGE_GESETZT.bin")),
            595445315
        );
    }

    #[test]
    fn fehlerfaelle() {
        assert_eq!(
            WDStyleCaptionPosition(std::ptr::null(), 0),
            ERR_BAD_ARGS
        );
        let junk = [0u8; 32];
        assert_eq!(caption_position(&junk), ERR_BAD_HEADER);
    }
}
