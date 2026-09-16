//! Layout-Detektor — Caption-/Icon-Position aus einem gerenderten Button-Bild.
//! ==========================================================================
//! Statt WinDevs verwuerfeltes ..Style-Format zu dekodieren, wird die Position
//! aus dem tatsaechlich gerenderten Button-Ausschnitt gelesen. Funktioniert
//! fuer JEDE Steuerung / jedes Gabarit / jede WinDev-Version.
//!
//! Verfahren (v2, komponentenbasiert):
//!   1. Hintergrund = haeufigste Farbe (Fuellung dominiert; robust gg. Rahmen).
//!   2. Vordergrund = |Pixel-bg|-Summe > THRESH; 2px-Rahmen ignoriert.
//!   3. 2D-Zusammenhangskomponenten (8er-Nachbarschaft), Mini-Rauschen weg.
//!   4. Trennung Text/Icon ueber die Hints (WinDev kennt beide Properties!):
//!        - image_hint=0            -> kein Icon, alles = Caption
//!        - caption_hint=0          -> Icon-only, alles = Icon
//!        - beide gesetzt           -> Icon = groesste KOMPAKTE Komponente,
//!                                     Text = alle uebrigen (Buchstaben).
//!      Funktioniert fuer Icon links/rechts/oben/unten, weil Komponenten
//!      2D getrennt werden (nicht nur per Spalten-Luecke).
//!   5. Caption wird RELATIV zur Restflaeche (Button minus Icon) klassifiziert
//!      -> "Center + rechtes Bild" ergibt korrekt caption=center.
//!
//! Eingabe: BMP-Bytes (24/32 Bit, BI_RGB) oder Rohpixel. WinDev: dSaveImageBMP.
//! Ausgabe: gepackter i32 (siehe pack()), negativ = Fehler.

pub const LAYOUT_ERR_ARGS: i32 = -1;
pub const LAYOUT_ERR_BMP: i32 = -2;

pub const P_NONE: i32 = 0;
pub const P_LEFT: i32 = 1; // = TOP auf der V-Achse
pub const P_CENTER: i32 = 2;
pub const P_RIGHT: i32 = 3; // = BOTTOM auf der V-Achse

const THRESH: i32 = 90;
const FRAME: usize = 2;

// ------------------------------- BMP-Parser --------------------------------

pub(crate) fn parse_bmp(d: &[u8]) -> Option<(usize, usize, Vec<[u8; 3]>)> {
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
    let bytes_pp = bpp / 8;
    let row_size = (w * bytes_pp + 3) & !3;
    if data_off + row_size * h > d.len() {
        return None;
    }
    let mut rgb = vec![[0u8; 3]; w * h];
    for y in 0..h {
        let src_row = if bottom_up { h - 1 - y } else { y };
        let base = data_off + src_row * row_size;
        for x in 0..w {
            let p = base + x * bytes_pp;
            rgb[y * w + x] = [d[p + 2], d[p + 1], d[p]]; // BGR -> RGB
        }
    }
    Some((w, h, rgb))
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

// ------------------------------- Detektor ----------------------------------

#[derive(Clone, Copy)]
struct Comp {
    x0: usize,
    y0: usize,
    x1: usize, // inklusiv
    y1: usize,
    area: usize, // Pixelanzahl
}
impl Comp {
    fn bw(&self) -> f32 {
        (self.x1 - self.x0 + 1) as f32
    }
    fn bh(&self) -> f32 {
        (self.y1 - self.y0 + 1) as f32
    }
    /// Icon-Score: gross + kompakt (aspect~1) + dicht gefuellt.
    fn icon_score(&self) -> f32 {
        let a = self.bw();
        let b = self.bh();
        let compact = a.min(b) / a.max(b); // 1 = quadratisch
        let density = self.area as f32 / (a * b); // 1 = voll gefuellt
        (self.area as f32).sqrt() * compact * (0.4 + density)
    }
}

fn dominant_bg(rgb: &[[u8; 3]]) -> [u8; 3] {
    let mut hist = std::collections::HashMap::<u32, u32>::new();
    for p in rgb {
        let key = ((p[0] as u32 >> 3) << 10) | ((p[1] as u32 >> 3) << 5) | (p[2] as u32 >> 3);
        *hist.entry(key).or_insert(0) += 1;
    }
    let key = hist.iter().max_by_key(|(_, c)| **c).map(|(k, _)| *k).unwrap_or(0);
    [
        ((((key >> 10) & 31) as u8) << 3) | 4,
        ((((key >> 5) & 31) as u8) << 3) | 4,
        (((key & 31) as u8) << 3) | 4,
    ]
}

/// 8er-Zusammenhangskomponenten der Vordergrundmaske (iterativer Flood-Fill).
fn components(fg: &[bool], w: usize, h: usize, min_area: usize) -> Vec<Comp> {
    let mut seen = vec![false; w * h];
    let mut out = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for sy in 0..h {
        for sx in 0..w {
            let si = sy * w + sx;
            if !fg[si] || seen[si] {
                continue;
            }
            let (mut x0, mut y0, mut x1, mut y1, mut area) = (sx, sy, sx, sy, 0usize);
            seen[si] = true;
            stack.push((sx, sy));
            while let Some((x, y)) = stack.pop() {
                area += 1;
                if x < x0 {
                    x0 = x;
                }
                if y < y0 {
                    y0 = y;
                }
                if x > x1 {
                    x1 = x;
                }
                if y > y1 {
                    y1 = y;
                }
                let (xl, xr) = (x.saturating_sub(1), (x + 1).min(w - 1));
                let (yt, yb) = (y.saturating_sub(1), (y + 1).min(h - 1));
                for ny in yt..=yb {
                    for nx in xl..=xr {
                        let ni = ny * w + nx;
                        if fg[ni] && !seen[ni] {
                            seen[ni] = true;
                            stack.push((nx, ny));
                        }
                    }
                }
            }
            if area >= min_area {
                out.push(Comp { x0, y0, x1, y1, area });
            }
        }
    }
    out
}

fn union(cs: &[&Comp]) -> Option<(usize, usize, usize, usize)> {
    if cs.is_empty() {
        return None;
    }
    let mut x0 = usize::MAX;
    let mut y0 = usize::MAX;
    let mut x1 = 0;
    let mut y1 = 0;
    for c in cs {
        x0 = x0.min(c.x0);
        y0 = y0.min(c.y0);
        x1 = x1.max(c.x1);
        y1 = y1.max(c.y1);
    }
    Some((x0, y0, x1, y1))
}

fn class_h(cx: f32) -> i32 {
    if cx < 0.40 {
        P_LEFT
    } else if cx > 0.60 {
        P_RIGHT
    } else {
        P_CENTER
    }
}
fn class_v(cy: f32) -> i32 {
    if cy < 0.40 {
        P_LEFT
    } else if cy > 0.60 {
        P_RIGHT
    } else {
        P_CENTER
    }
}

/// (display_caption, cap_h, cap_v, has_image, img_h, img_v)
fn analyze(
    w: usize,
    h: usize,
    rgb: &[[u8; 3]],
    caption_hint: i32,
    image_hint: i32,
) -> (bool, i32, i32, bool, i32, i32) {
    let none = (false, P_NONE, P_NONE, false, P_NONE, P_NONE);
    if w < 6 || h < 6 {
        return none;
    }
    let bg = dominant_bg(rgb);

    let mut fg = vec![false; w * h];
    let mut fg_count = 0usize;
    for y in FRAME..h.saturating_sub(FRAME) {
        for x in FRAME..w.saturating_sub(FRAME) {
            let px = rgb[y * w + x];
            let dist = (px[0] as i32 - bg[0] as i32).abs()
                + (px[1] as i32 - bg[1] as i32).abs()
                + (px[2] as i32 - bg[2] as i32).abs();
            if dist > THRESH {
                fg[y * w + x] = true;
                fg_count += 1;
            }
        }
    }
    if fg_count < 8 {
        return none;
    }

    let min_area = core::cmp::max(4, (w * h) / 3000);
    let comps = components(&fg, w, h, min_area);
    if comps.is_empty() {
        return none;
    }

    let want_cap = caption_hint != 0; // 1 oder -1
    let want_img = image_hint != 0;

    // Icon-Komponente bestimmen
    let icon_idx: Option<usize> = if !want_img {
        None
    } else if !want_cap {
        // Icon-only: alle Vordergrund-Komponenten = Icon (nimm groessten Score
        // nur zur Positionsbestimmung ueber die Vereinigung).
        None // Marker: Icon = Vereinigung ALLER Komponenten (siehe unten)
    } else {
        // Beide vorhanden: Icon = Komponente mit hoechstem Icon-Score.
        let mut best = 0usize;
        for i in 1..comps.len() {
            if comps[i].icon_score() > comps[best].icon_score() {
                best = i;
            }
        }
        Some(best)
    };

    // Icon-Bounding-Box + Position (ueber den ganzen Button)
    let (has_image, img_h, img_v, icon_bb) = if !want_img {
        (false, P_NONE, P_NONE, None)
    } else if !want_cap {
        // Icon-only: Vereinigung aller Komponenten
        let refs: Vec<&Comp> = comps.iter().collect();
        let bb = union(&refs).unwrap();
        let cx = (bb.0 + bb.2) as f32 / 2.0 / w as f32;
        let cy = (bb.1 + bb.3) as f32 / 2.0 / h as f32;
        (true, class_h(cx), class_v(cy), Some(bb))
    } else {
        let c = &comps[icon_idx.unwrap()];
        let cx = (c.x0 + c.x1) as f32 / 2.0 / w as f32;
        let cy = (c.y0 + c.y1) as f32 / 2.0 / h as f32;
        (true, class_h(cx), class_v(cy), Some((c.x0, c.y0, c.x1, c.y1)))
    };

    // Text-Bounding-Box = alle Nicht-Icon-Komponenten
    let text_bb = if !want_cap {
        None
    } else {
        let refs: Vec<&Comp> = comps
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != icon_idx)
            .map(|(_, c)| c)
            .collect();
        union(&refs)
    };

    // Caption RELATIV zur Restflaeche (Button minus Icon-Bereich) klassifizieren
    let (has_caption, cap_h, cap_v) = match text_bb {
        None => (false, P_NONE, P_NONE),
        Some((tx0, ty0, tx1, ty1)) => {
            let (mut ax0, mut ay0, mut ax1, mut ay1) = (0usize, 0usize, w, h);
            if let Some((ix0, iy0, ix1, iy1)) = icon_bb {
                if img_h == P_RIGHT {
                    ax1 = ix0;
                } else if img_h == P_LEFT {
                    ax0 = ix1 + 1;
                }
                if img_v == P_RIGHT {
                    ay1 = iy0;
                } else if img_v == P_LEFT {
                    ay0 = iy1 + 1;
                }
            }
            let aw = (ax1.saturating_sub(ax0)).max(1) as f32;
            let ah = (ay1.saturating_sub(ay0)).max(1) as f32;
            let cx = ((tx0 + tx1) as f32 / 2.0 - ax0 as f32) / aw;
            let cy = ((ty0 + ty1) as f32 / 2.0 - ay0 as f32) / ah;
            (true, class_h(cx.clamp(0.0, 1.0)), class_v(cy.clamp(0.0, 1.0)))
        }
    };

    (has_caption, cap_h, cap_v, has_image, img_h, img_v)
}

/// bits 0-1: caption_h  2-3: caption_v  4: display_caption
/// bits 5-6: image_h    7-8: image_v    9: has_image
fn pack(r: (bool, i32, i32, bool, i32, i32)) -> i32 {
    let (dc, ch, cv, hi, ih, iv) = r;
    (ch & 3)
        | ((cv & 3) << 2)
        | ((dc as i32) << 4)
        | ((ih & 3) << 5)
        | ((iv & 3) << 7)
        | ((hi as i32) << 9)
}

/// Gleiche Packung, fuer layout3 (v3) wiederverwendet.
pub(crate) fn pack_result(dc: bool, ch: i32, cv: i32, hi: bool, ih: i32, iv: i32) -> i32 {
    pack((dc, ch, cv, hi, ih, iv))
}

// ------------------------------- FFI ---------------------------------------

/// Analysiert einen Button-Ausschnitt (BMP-Bytes).
/// caption_hint / image_hint: 1 = vorhanden, 0 = nicht vorhanden, -1 = ableiten.
/// WinDev kennt beide Properties -> am besten 1/0 uebergeben.
/// WLanguage: nP = API("wd_style64.dll","WDLayoutAnalyzeBMP", buf, Length(buf), nCap, nImg)
#[no_mangle]
pub extern "C" fn WDLayoutAnalyzeBMP(
    data: *const u8,
    len: i32,
    caption_hint: i32,
    image_hint: i32,
) -> i32 {
    if data.is_null() || len < 54 {
        return LAYOUT_ERR_ARGS;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    let (w, h, rgb) = match parse_bmp(slice) {
        Some(v) => v,
        None => return LAYOUT_ERR_BMP,
    };
    pack(analyze(w, h, &rgb, caption_hint, image_hint))
}

/// Wie WDLayoutAnalyzeBMP, aber mit rohen Pixeln.
/// fmt: 0=RGB, 1=BGR (3 B/px), 2=BGRA, 3=RGBA (4 B/px). stride=0 => dicht.
#[no_mangle]
pub extern "C" fn WDLayoutAnalyzeRaw(
    data: *const u8,
    width: i32,
    height: i32,
    stride: i32,
    fmt: i32,
    caption_hint: i32,
    image_hint: i32,
) -> i32 {
    if data.is_null() || width <= 0 || height <= 0 {
        return LAYOUT_ERR_ARGS;
    }
    let (w, h) = (width as usize, height as usize);
    let bpp = if fmt >= 2 { 4 } else { 3 };
    let stride = if stride > 0 { stride as usize } else { w * bpp };
    let slice = unsafe { std::slice::from_raw_parts(data, stride * h) };
    let mut rgb = vec![[0u8; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let p = y * stride + x * bpp;
            rgb[y * w + x] = match fmt {
                1 | 2 => [slice[p + 2], slice[p + 1], slice[p]],
                _ => [slice[p], slice[p + 1], slice[p + 2]],
            };
        }
    }
    pack(analyze(w, h, &rgb, caption_hint, image_hint))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn run(dir: &str, file: &str, cap: i32, img: i32) -> (bool, i32, i32, bool, i32, i32) {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(dir).join(file);
        let d = fs::read(&p).unwrap_or_else(|_| panic!("fehlt: {}", file));
        let packed = WDLayoutAnalyzeBMP(d.as_ptr(), d.len() as i32, cap, img);
        assert!(packed >= 0, "Fehler {}", packed);
        (
            (packed >> 4) & 1 == 1,
            packed & 3,
            (packed >> 2) & 3,
            (packed >> 9) & 1 == 1,
            (packed >> 5) & 3,
            (packed >> 7) & 3,
        )
    }

    #[test]
    fn echte_buttons() {
        let dir = "button_images/bmp";
        // SEPA Import: Caption zentriert, kein Icon
        let r = run(dir, "Dienstprogramme_Import_Baeckerei_2003.BTN_SEPA.bmp", 1, 0);
        assert_eq!((r.0, r.1, r.2, r.3), (true, P_CENTER, P_CENTER, false));
        // Start: Caption center (im Restbereich), Icon rechts
        let r = run(dir, "Dienstprogramme_Import_Baeckerei_2003.BTN_Start.bmp", 1, 1);
        assert_eq!((r.0, r.1, r.3, r.4), (true, P_CENTER, true, P_RIGHT));
        // Abbrechen: Caption center, Icon rechts
        let r = run(dir, "Dienstprogramme_Import_Baeckerei_2003.Button2.bmp", 1, 1);
        assert_eq!((r.0, r.1, r.3, r.4), (true, P_CENTER, true, P_RIGHT));
    }
}
