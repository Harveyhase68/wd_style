//! Button-Zustandsbild: überzählige Steps entfernen.
//! ==================================================================
//! WinDev-Buttons haben ein Hintergrundbild mit S *States* (nebeneinander,
//! horizontal) und P *Steps* (übereinander, vertikal gestapelt — jede Step-Zeile
//! ein Band). Die Step-Zahl ist per WLanguage NICHT auslesbar, die State-Zahl (1..5)
//! schon.
//!
//! Beobachtung an 66 Demo-Bildern: es gibt praktisch nur **1 Step oder 6 Steps**.
//! Über die Bildgröße allein NICHT unterscheidbar (ein 1-Step-Button kann 400x144
//! sein wie ein 6-Step-Button). Aber die ZELL-Proportion trennt sauber:
//!
//! ```text
//! cell = Breite / (States * Höhe)
//!   1 Step : cell >= 0.7   (Zelle breit/quadratisch)  -> Bild unverändert
//!   6 Steps: cell <  0.7   (Zelle 6x zu flach)         -> auf oberstes 1/6 kürzen
//! ```
//!
//! (Messwerte: 1-Step cell-min 0.80, 6-Step cell-max 0.56 -> Schwelle 0.7 mittig.)

pub const STEPS_ERR_ARGS: i32 = -1;
pub const STEPS_ERR_BMP: i32 = -2;

const CELL_THRESHOLD: f32 = 0.7;
const MULTI_STEPS: usize = 6; // laut Datenlage sind Mehr-Step-Bilder immer 6 Steps

// --- BMP roh (Pixel unverändert BGR/BGRA, top-down Zeilen) -----------------

struct Bmp {
    w: usize,
    h: usize,
    bpp: usize,           // 24 oder 32
    rows: Vec<Vec<u8>>,   // top-down; je Zeile w*bpp/8 Bytes (BGR/BGRA wie in Datei)
}

fn u16_le(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}
fn u32_le(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}
fn i32_le(d: &[u8], o: usize) -> i32 {
    i32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

fn parse_bmp(d: &[u8]) -> Option<Bmp> {
    if d.len() < 54 || &d[0..2] != b"BM" {
        return None;
    }
    let data_off = u32_le(d, 10) as usize;
    let width = i32_le(d, 18);
    let height_raw = i32_le(d, 22);
    let bpp = u16_le(d, 28) as usize;
    let compression = u32_le(d, 30);
    if compression != 0 || (bpp != 24 && bpp != 32) || width <= 0 || height_raw == 0 {
        return None;
    }
    let w = width as usize;
    let bottom_up = height_raw > 0;
    let h = height_raw.unsigned_abs() as usize;
    let bpr = bpp / 8;
    let row_size = (w * bpr + 3) & !3;
    if data_off + row_size * h > d.len() {
        return None;
    }
    let mut rows = Vec::with_capacity(h);
    for y in 0..h {
        let src = if bottom_up { h - 1 - y } else { y };
        let base = data_off + src * row_size;
        rows.push(d[base..base + w * bpr].to_vec());
    }
    Some(Bmp { w, h, bpp, rows })
}

/// Schreibt ein bottom-up-BMP aus top-down-Zeilen.
fn write_bmp(w: usize, h: usize, bpp: usize, rows: &[Vec<u8>]) -> Vec<u8> {
    let bpr = bpp / 8;
    let row_size = (w * bpr + 3) & !3;
    let data_size = row_size * h;
    let file_size = 54 + data_size;
    let mut o = Vec::with_capacity(file_size);
    o.extend_from_slice(b"BM");
    o.extend_from_slice(&(file_size as u32).to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes()); // reserved
    o.extend_from_slice(&54u32.to_le_bytes()); // data offset
    o.extend_from_slice(&40u32.to_le_bytes()); // DIB header size
    o.extend_from_slice(&(w as i32).to_le_bytes());
    o.extend_from_slice(&(h as i32).to_le_bytes()); // positiv = bottom-up
    o.extend_from_slice(&1u16.to_le_bytes()); // planes
    o.extend_from_slice(&(bpp as u16).to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    o.extend_from_slice(&(data_size as u32).to_le_bytes());
    o.extend_from_slice(&2835i32.to_le_bytes()); // 72 dpi
    o.extend_from_slice(&2835i32.to_le_bytes());
    o.extend_from_slice(&0u32.to_le_bytes()); // clrused
    o.extend_from_slice(&0u32.to_le_bytes()); // clrimp
    let pad = row_size - w * bpr;
    for r in 0..h {
        let src = h - 1 - r; // bottom-up schreiben
        o.extend_from_slice(&rows[src]);
        o.extend(std::iter::repeat(0u8).take(pad));
    }
    o
}

/// Anzahl Steps (1 oder MULTI_STEPS) aus Größe + State-Zahl.
fn step_count(w: usize, h: usize, num_states: i32) -> usize {
    let s = num_states.max(1) as f32;
    let cell = w as f32 / (s * h as f32);
    if cell >= CELL_THRESHOLD {
        1
    } else {
        MULTI_STEPS
    }
}

// ------------------------------- FFI ---------------------------------------

/// Liefert die erkannte Step-Anzahl (1 oder 6) eines Button-Zustandsbildes (BMP).
/// numStates = Anzahl der States (WinDev-Property, 1..5). Negativ = Fehler.
/// WLanguage: n = API("wd_style64.dll","WDImageStepCount", bufBMP, Length(bufBMP), nStates)
#[no_mangle]
pub extern "C" fn WDImageStepCount(data: *const u8, len: i32, num_states: i32) -> i32 {
    if data.is_null() || len < 54 {
        return STEPS_ERR_ARGS;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    match parse_bmp(slice) {
        Some(b) => step_count(b.w, b.h, num_states) as i32,
        None => STEPS_ERR_BMP,
    }
}

/// Reduziert ein Mehr-Step-Bild auf 1 Step (behält das oberste Band, volle Breite).
/// 1-Step-Bilder bleiben UNVERÄNDERT. Schreibt das Ergebnis-BMP nach `out`
/// (bis `out_cap`); mit out=NULL/out_cap=0 nur die nötige Größe abfragen.
/// Rückgabe: Länge des Ergebnis-BMP (immer, auch wenn out zu klein). Negativ = Fehler.
/// WLanguage:
///   nLen is int = API(sDLL,"WDImageReduceSteps", bufBMP, Length(bufBMP), nStates, Null, 0)
///   bufOut is Buffer = RepeatString(Charact(0), nLen)
///   API(sDLL,"WDImageReduceSteps", bufBMP, Length(bufBMP), nStates, &bufOut, nLen)
#[no_mangle]
pub extern "C" fn WDImageReduceSteps(
    data: *const u8,
    len: i32,
    num_states: i32,
    out: *mut u8,
    out_cap: i32,
) -> i32 {
    if data.is_null() || len < 54 {
        return STEPS_ERR_ARGS;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    let b = match parse_bmp(slice) {
        Some(b) => b,
        None => return STEPS_ERR_BMP,
    };
    let steps = step_count(b.w, b.h, num_states);
    let result: Vec<u8> = if steps <= 1 {
        slice.to_vec() // 1 Step -> unverändert zurück
    } else {
        let new_h = (b.h / steps).max(1); // oberstes Band
        write_bmp(b.w, new_h, b.bpp, &b.rows[..new_h])
    };
    if !out.is_null() && out_cap > 0 {
        let n = result.len().min(out_cap as usize);
        unsafe { std::ptr::copy_nonoverlapping(result.as_ptr(), out, n) };
    }
    result.len() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal-BMP (24-bit, uni-Farbe) bauen zum Testen
    fn make_bmp(w: usize, h: usize) -> Vec<u8> {
        let rows: Vec<Vec<u8>> = (0..h)
            .map(|y| {
                let mut r = Vec::with_capacity(w * 3);
                for _ in 0..w {
                    r.extend_from_slice(&[y as u8, 0, 0]); // Zeilennummer als Blau
                }
                r
            })
            .collect();
        write_bmp(w, h, 24, &rows)
    }

    #[test]
    fn erkennt_und_kuerzt() {
        // 5 States, 6 Steps: 400x144 -> cell = 400/(5*144)=0.55 < 0.7 -> 6 Steps
        let big = make_bmp(400, 144);
        assert_eq!(WDImageStepCount(big.as_ptr(), big.len() as i32, 5), 6);
        let need = WDImageReduceSteps(big.as_ptr(), big.len() as i32, 5, std::ptr::null_mut(), 0);
        let mut out = vec![0u8; need as usize];
        WDImageReduceSteps(big.as_ptr(), big.len() as i32, 5, out.as_mut_ptr(), need);
        let red = parse_bmp(&out).unwrap();
        assert_eq!((red.w, red.h), (400, 24)); // auf oberstes 1/6 gekürzt
        assert_eq!(red.rows[0][0], 0); // oberste Zeile = Step 0

        // 5 States, 1 Step: 400x24 -> cell = 400/120 = 3.33 -> 1 Step, unverändert
        let one = make_bmp(400, 24);
        assert_eq!(WDImageStepCount(one.as_ptr(), one.len() as i32, 5), 1);
        let need = WDImageReduceSteps(one.as_ptr(), one.len() as i32, 5, std::ptr::null_mut(), 0);
        assert_eq!(need as usize, one.len()); // gleiche Größe -> unverändert
    }
}
