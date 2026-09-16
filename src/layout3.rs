//! Layout-Detektor v3 — erkennt jetzt auch die Caption-PRAESENZ selbst.
//! ====================================================================
//! Port von layout_v3_prototyp.py (validiert an 207 gelabelten Buttons der
//! Themes Eleven/Cobalt/Ankaa: Caption-Praesenz 206/207, Icon-Seite ~93%).
//!
//! Unterschied zu v2 (layout.rs):
//!  - caption-Hint bedeutet nur noch "Caption-TEXT ist nicht leer". Ob die
//!    Caption ANGEZEIGT wird, erkennt der Detektor selbst (Buchstabenketten).
//!    -> Ein Icon-only-Button liefert korrekt "keine Caption", auch wenn die
//!    Caption-Property Text enthaelt (der beruechtigte Button3-Fall).
//!  - Der Caption-STRING wird uebergeben: aus seiner Glyphenzahl erkennt der
//!    Detektor am Text klebende Juxtaposed-Icons (ueberzaehlige Kettenglieder).
//!  - Fette kleine Schrift, die zu Woertern verschmilzt (Ankaa!), wird ueber
//!    breite flache "Wort-Blobs" erkannt.
//!  - Zusatz-Exporte: Hintergrundfarbe (Fuellung), Schriftfarbe, hell/dunkel.
//!
//! Bekannte Grenze (naechster Ausbauschritt): Icon in einer Farbe, die
//! gleichzeitig dominante Hintergrundfarbe ist (weisses Icon auf hellem Fill
//! mit weissen Fensterecken, Cobalt-NC-Buttons) -> Loesung waere
//! Konnektivitaet pro Farbregion (Inseln = Tinte). Ein naiver Flood-Fill vom
//! Rand ist KEINE Loesung (sickert durch Anti-Aliasing, im Prototyp getestet
//! und verworfen).

use crate::layout::{pack_result, parse_bmp, P_CENTER, P_LEFT, P_NONE, P_RIGHT};

pub const LAYOUT_ERR_ARGS: i32 = -1;
pub const LAYOUT_ERR_BMP: i32 = -2;
pub const LAYOUT_ERR_NO_TEXT: i32 = -3;

// ------------------------------ Komponenten --------------------------------

#[derive(Clone)]
struct C {
    x0: i64,
    y0: i64,
    x1: i64, // inklusiv
    y1: i64,
    n: usize,
    px: Vec<(u32, u32)>, // (y, x)
}
impl C {
    fn w(&self) -> i64 {
        self.x1 - self.x0 + 1
    }
    fn h(&self) -> i64 {
        self.y1 - self.y0 + 1
    }
    fn dens(&self) -> f64 {
        self.n as f64 / (self.w() as f64 * self.h() as f64)
    }
}

/// Dominante Farben (quantisiert auf 32er-Stufen), Anteil >= 6 %.
/// Rueckgabe: (Mittelwertfarbe, Pixelzahl) absteigend nach Haeufigkeit.
fn dominant_colors(rgb: &[[u8; 3]]) -> Vec<([f64; 3], usize)> {
    let mut count = [0usize; 512];
    let mut sum = [[0u64; 3]; 512];
    for p in rgb {
        let key = ((p[0] as usize >> 5) << 6) | ((p[1] as usize >> 5) << 3) | (p[2] as usize >> 5);
        count[key] += 1;
        sum[key][0] += p[0] as u64;
        sum[key][1] += p[1] as u64;
        sum[key][2] += p[2] as u64;
    }
    let total = rgb.len();
    let mut bins: Vec<usize> = (0..512).filter(|&k| count[k] > 0).collect();
    // absteigend nach Anzahl, bei Gleichstand kleiner Key zuerst (deterministisch)
    bins.sort_by(|&a, &b| count[b].cmp(&count[a]).then(a.cmp(&b)));
    let mut out = Vec::new();
    for k in bins {
        if (count[k] as f64) / (total as f64) < 0.06 {
            break;
        }
        let m = [
            sum[k][0] as f64 / count[k] as f64,
            sum[k][1] as f64 / count[k] as f64,
            sum[k][2] as f64 / count[k] as f64,
        ];
        out.push((m, count[k]));
    }
    out
}

/// Tinte = Pixel weit weg von allen dominanten Hintergrundfarben.
fn ink_mask(w: usize, h: usize, rgb: &[[u8; 3]]) -> (Vec<bool>, [f64; 3]) {
    let doms = dominant_colors(rgb);
    if doms.is_empty() {
        return (vec![false; w * h], [255.0, 255.0, 255.0]);
    }
    let mut dist = vec![f64::MAX; w * h];
    let mut dmax = 0.0f64;
    for (i, p) in rgb.iter().enumerate() {
        let mut best = f64::MAX;
        for (m, _) in &doms {
            let d = (p[0] as f64 - m[0]).abs() + (p[1] as f64 - m[1]).abs()
                + (p[2] as f64 - m[2]).abs();
            if d < best {
                best = d;
            }
        }
        dist[i] = best;
        if best > dmax {
            dmax = best;
        }
    }
    let thr = (dmax * 0.35).max(90.0);
    let mask: Vec<bool> = dist.iter().map(|&d| d > thr).collect();
    (mask, doms[0].0)
}

/// Fallback, wenn die Dominanzfarben-Maske nichts findet: Tinte = Regionen,
/// die einer dominanten Farbe angehoeren, aber den BILDRAND NICHT beruehren
/// (Inseln). Loest "weisses Icon auf hellem Fill neben weissen Fensterecken".
/// Kein lokaler Flood-Fill -> sickert nicht durch Anti-Aliasing.
fn ink_mask_islands(w: usize, h: usize, rgb: &[[u8; 3]]) -> Vec<bool> {
    let doms = dominant_colors(rgb);
    let mut bg = vec![false; w * h];
    for (m, _) in &doms {
        let mask_d: Vec<bool> = rgb
            .iter()
            .map(|p| {
                (p[0] as f64 - m[0]).abs() + (p[1] as f64 - m[1]).abs()
                    + (p[2] as f64 - m[2]).abs()
                    <= 60.0
            })
            .collect();
        for c in components(&mask_d, w, h) {
            if c.x0 == 0 || c.y0 == 0 || c.x1 == w as i64 - 1 || c.y1 == h as i64 - 1 {
                for &(y, x) in &c.px {
                    bg[y as usize * w + x as usize] = true;
                }
            }
        }
    }
    bg.iter().map(|&b| !b).collect()
}

/// 8er-Zusammenhangskomponenten (Scanreihenfolge = Python-Prototyp).
fn components(mask: &[bool], w: usize, h: usize) -> Vec<C> {
    let mut lab = vec![false; w * h];
    let mut comps = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for sy in 0..h {
        for sx in 0..w {
            let si = sy * w + sx;
            if !mask[si] || lab[si] {
                continue;
            }
            lab[si] = true;
            stack.push((sy, sx));
            let mut px: Vec<(u32, u32)> = Vec::new();
            while let Some((y, x)) = stack.pop() {
                px.push((y as u32, x as u32));
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (ny, nx) = (y as i64 + dy, x as i64 + dx);
                        if ny >= 0 && (ny as usize) < h && nx >= 0 && (nx as usize) < w {
                            let ni = ny as usize * w + nx as usize;
                            if mask[ni] && !lab[ni] {
                                lab[ni] = true;
                                stack.push((ny as usize, nx as usize));
                            }
                        }
                    }
                }
            }
            let x0 = px.iter().map(|p| p.1).min().unwrap() as i64;
            let x1 = px.iter().map(|p| p.1).max().unwrap() as i64;
            let y0 = px.iter().map(|p| p.0).min().unwrap() as i64;
            let y1 = px.iter().map(|p| p.0).max().unwrap() as i64;
            comps.push(C { x0, y0, x1, y1, n: px.len(), px });
        }
    }
    comps
}

/// Erwartete Komponentenzahl eines gerenderten ANSI-Texts (i/j/Umlaut-Punkte
/// zaehlen als eigene Komponente).
fn expected_glyphs(caption: &[u8]) -> usize {
    let mut n = 0usize;
    for &b in caption {
        match b {
            0x20 | 0x09..=0x0D | 0xA0 => continue, // Leerraum
            _ => {}
        }
        n += 1;
        match b {
            b'i' | b'j' | b'!' | b'?' | b':' | b';' | b'=' => n += 1,
            0xE4 | 0xF6 | 0xFC | 0xC4 | 0xD6 | 0xDC => n += 1, // ae oe ue (CP1252)
            _ => {}
        }
    }
    n
}

/// Nicht-Ketten-Komponenten zu Icon-Clustern verschmelzen (BBox-Abstand <= gap).
fn cluster_boxes(mut cl: Vec<C>, gap: i64) -> Vec<C> {
    let mut changed = true;
    while changed {
        changed = false;
        'outer: for i in 0..cl.len() {
            for j in (i + 1)..cl.len() {
                let (a, b) = (&cl[i], &cl[j]);
                if a.x0 - gap <= b.x1 && b.x0 - gap <= a.x1 && a.y0 - gap <= b.y1
                    && b.y0 - gap <= a.y1
                {
                    let b = cl.remove(j);
                    let a = &mut cl[i];
                    a.x0 = a.x0.min(b.x0);
                    a.x1 = a.x1.max(b.x1);
                    a.y0 = a.y0.min(b.y0);
                    a.y1 = a.y1.max(b.y1);
                    a.n += b.n;
                    a.px.extend_from_slice(&b.px);
                    changed = true;
                    break 'outer;
                }
            }
        }
    }
    cl
}

fn median_color(rgb: &[[u8; 3]], w: usize, px: &[(u32, u32)]) -> [f64; 3] {
    let mut out = [0.0f64; 3];
    for ch in 0..3 {
        let mut v: Vec<u8> = px.iter().map(|&(y, x)| rgb[y as usize * w + x as usize][ch]).collect();
        v.sort_unstable();
        let m = v.len() / 2;
        out[ch] = if v.len() % 2 == 1 {
            v[m] as f64
        } else {
            (v[m - 1] as f64 + v[m] as f64) / 2.0
        };
    }
    out
}

// ------------------------------- Ergebnis ----------------------------------

pub struct Layout3 {
    pub cap_present: bool,
    pub cap_h: i32,
    pub cap_v: i32,
    pub img_present: bool,
    pub img_h: i32,
    pub img_v: i32,
    pub bg: [f64; 3],
    pub dark: bool,
    pub fg: Option<[f64; 3]>,
}

fn h_class(cx: f64, x0: f64, x1: f64) -> i32 {
    let rel = (cx - x0) / (x1 - x0).max(1.0);
    if rel < 0.38 {
        P_LEFT
    } else if rel > 0.62 {
        P_RIGHT
    } else {
        P_CENTER
    }
}
fn v_class(cy: f64, y0: f64, y1: f64) -> i32 {
    let rel = (cy - y0) / (y1 - y0).max(1.0);
    if rel < 0.35 {
        P_LEFT
    } else if rel > 0.65 {
        P_RIGHT
    } else {
        P_CENTER
    }
}

/// Kern-Analyse. caption: None/leer = Caption-Property ist leer;
/// sonst der Caption-Text (ANSI-Bytes). image_hint = Bild-Property belegt.
pub fn analyze3(w: usize, h: usize, rgb: &[[u8; 3]], caption: Option<&[u8]>, image_hint: bool) -> Layout3 {
    let (mask, fill) = ink_mask(w, h, rgb);
    let dark = fill[0] * 0.299 + fill[1] * 0.587 + fill[2] * 0.114 < 128.0;
    let mut res = Layout3 {
        cap_present: false,
        cap_h: P_NONE,
        cap_v: P_NONE,
        img_present: false,
        img_h: P_NONE,
        img_v: P_NONE,
        bg: fill,
        dark,
        fg: None,
    };
    let caption_hint = caption.map(|c| !c.is_empty()).unwrap_or(false);
    let wf = w as f64;
    let hf = h as f64;

    // Rahmen/Rand ausblenden: 2px umlaufend
    let mut mask2 = mask;
    for y in 0..h {
        for x in 0..w {
            if y < 2 || y >= h.saturating_sub(2) || x < 2 || x >= w.saturating_sub(2) {
                mask2[y * w + x] = false;
            }
        }
    }

    let build_comps = |m: &[bool]| -> Vec<C> {
        let mut cs: Vec<C> = components(m, w, h).into_iter().filter(|c| c.n >= 4).collect();
        // Rahmenreste (fast ganze Flaeche) und Glow-Linien filtern
        cs.retain(|c| !((c.x1 - c.x0) as f64 > 0.92 * wf && (c.y1 - c.y0) as f64 > 0.92 * hf));
        cs.retain(|c| {
            let lw = c.w();
            let lh = c.h();
            !((lh <= 3 && lw as f64 >= 0.5 * wf) || (lw <= 3 && lh as f64 >= 0.5 * hf))
        });
        cs
    };
    let mut comps = build_comps(&mask2);
    if comps.is_empty() {
        // Fallback: Insel-Segmentierung (Icon-Farbe == dominante Hintergrundfarbe)
        let mut mask3 = ink_mask_islands(w, h, rgb);
        for y in 0..h {
            for x in 0..w {
                if y < 2 || y >= h.saturating_sub(2) || x < 2 || x >= w.saturating_sub(2) {
                    mask3[y * w + x] = false;
                }
            }
        }
        comps = build_comps(&mask3);
    }
    if comps.is_empty() {
        return res;
    }

    // ---- Textketten: buchstabenartige Komponenten gruppieren ---------------
    let mut letter_idx: Vec<usize> = (0..comps.len())
        .filter(|&i| {
            let c = &comps[i];
            c.h() >= 4 && (c.h() as f64) <= 0.95 * hf && (c.w() as f64) <= 2.4 * c.h() as f64
        })
        .collect();
    letter_idx.sort_by_key(|&i| comps[i].x0); // stabil wie Python sorted()

    let mut chains: Vec<Vec<usize>> = Vec::new();
    for &ci in &letter_idx {
        let c = &comps[ci];
        let mut placed = false;
        for ch in chains.iter_mut() {
            let last = &comps[*ch.last().unwrap()];
            let ovl = c.y1.min(last.y1) - c.y0.max(last.y0) + 1;
            let hmin = c.h().min(last.h());
            let hmax = c.h().max(last.h());
            let gap = c.x0 - last.x1;
            if ovl as f64 >= 0.5 * hmin as f64
                && hmax as f64 <= 1.9 * hmin as f64
                && gap >= -2
                && gap as f64 <= 1.3 * hmax as f64
            {
                ch.push(ci);
                placed = true;
                break;
            }
        }
        if !placed {
            chains.push(vec![ci]);
        }
    }
    chains.sort_by(|a, b| b.len().cmp(&a.len())); // stabil

    let mut text_chain: Option<Vec<usize>> = None;
    let mut stripped: Vec<usize> = Vec::new();
    if caption_hint && !image_hint {
        // Kein Bild zugewiesen -> ALLES Gerenderte ist die Caption. Wichtig
        // fuer Ein-/Zwei-Zeichen-Captions ("<", ">>", Navigations-Buttons),
        // die keine Buchstabenkette bilden koennen.
        text_chain = Some((0..comps.len()).collect());
    } else if caption_hint {
        let exp0 = caption.map(expected_glyphs);
        // Weg B: verschmolzener Text = breiter flacher LOECHRIGER Blob
        let mut blob: Option<usize> = None;
        for (i, c) in comps.iter().enumerate() {
            if c.w() as f64 >= 2.2 * c.h() as f64
                && c.w() >= 12
                && c.h() >= 4
                && (c.h() as f64) <= 0.7 * hf
                && c.dens() <= 0.75
            {
                if blob.map(|b| c.n > comps[b].n).unwrap_or(true) {
                    blob = Some(i);
                }
            }
        }
        if !chains.is_empty() && chains[0].len() >= 3 {
            text_chain = Some(chains[0].clone());
        } else if !chains.is_empty() && chains[0].len() == 2 {
            let rest = (0..comps.len()).any(|i| !chains[0].contains(&i));
            let cw2 = chains[0].iter().map(|&i| comps[i].x1).max().unwrap()
                - chains[0].iter().map(|&i| comps[i].x0).min().unwrap()
                + 1;
            let ch2 = chains[0].iter().map(|&i| comps[i].h()).max().unwrap();
            let wordy = cw2 as f64 >= 2.2 * ch2 as f64
                && cw2 >= 16
                && chains[0].iter().all(|&i| {
                    comps[i].n as f64 <= 0.85 * comps[i].w() as f64 * comps[i].h() as f64
                });
            if !rest || wordy || exp0.map(|e| e <= 3).unwrap_or(false) {
                text_chain = Some(chains[0].clone());
            }
        }
        // Blob gewinnt, wenn keine (oder eine schmalere) Kette gefunden wurde
        if let Some(bi) = blob {
            let chain_w = text_chain
                .as_ref()
                .map(|tc| {
                    tc.iter().map(|&i| comps[i].x1).max().unwrap()
                        - tc.iter().map(|&i| comps[i].x0).min().unwrap()
                })
                .unwrap_or(0);
            let in_chain = text_chain.as_ref().map(|tc| tc.contains(&bi)).unwrap_or(false);
            if text_chain.is_none() || (!in_chain && comps[bi].w() as f64 > 1.2 * chain_w as f64) {
                text_chain = Some(vec![bi]);
            }
        }
    }

    // ---- Juxtaposed-Icon von der Kette abspalten ---------------------------
    if let (Some(tc), Some(cap), true) = (text_chain.as_mut(), caption, image_hint) {
        let exp = expected_glyphs(cap);
        let mut heights: Vec<i64> = tc.iter().map(|&i| comps[i].h()).collect();
        heights.sort_unstable();
        let med = heights[heights.len() / 2];
        while tc.len() > exp.max(1) {
            let first = &comps[tc[0]];
            let last = &comps[*tc.last().unwrap()];
            let fdev = (first.h() - med).abs();
            let ldev = (last.h() - med).abs();
            let (cand_h, front) = if fdev >= ldev { (first.h(), true) } else { (last.h(), false) };
            let lim = (0.2 * med as f64).max(2.0);
            if (cand_h - med).abs() as f64 > lim {
                if front {
                    stripped.push(tc.remove(0));
                } else {
                    stripped.push(tc.pop().unwrap());
                }
                continue;
            }
            // Fallback 2: Luecken-Ausreisser am Kettenende
            if tc.len() >= 3 {
                let gaps: Vec<i64> = (0..tc.len() - 1)
                    .map(|i| comps[tc[i + 1]].x0 - comps[tc[i]].x1)
                    .collect();
                let mut inner: Vec<i64> = if gaps.len() > 2 {
                    gaps[1..gaps.len() - 1].to_vec()
                } else {
                    gaps.clone()
                };
                inner.sort_unstable();
                let medgap = inner[inner.len() / 2];
                let lim2 = 2.0 + 1.6 * medgap as f64;
                if gaps[0] as f64 >= lim2 && gaps[0] >= *gaps.last().unwrap() {
                    stripped.push(tc.remove(0));
                    continue;
                }
                if *gaps.last().unwrap() as f64 >= lim2 {
                    stripped.push(tc.pop().unwrap());
                    continue;
                }
            }
            // Fallback 3: quadratisches Endglied (nur bei Ketten-UEBERlaenge)
            let sq = |c: &C| ((c.w() - c.h()).abs() as f64) <= (0.25 * c.h() as f64).max(2.0);
            let fsq = sq(first);
            let lsq = sq(last);
            if fsq && !lsq {
                stripped.push(tc.remove(0));
                continue;
            }
            if lsq && !fsq {
                stripped.push(tc.pop().unwrap());
                continue;
            }
            break;
        }
        if tc.is_empty() {
            text_chain = None;
        }
    }

    let mut text_box: Option<(i64, i64, i64, i64)> = None;
    if let Some(tc) = &text_chain {
        res.cap_present = true;
        let tx0 = tc.iter().map(|&i| comps[i].x0).min().unwrap();
        let tx1 = tc.iter().map(|&i| comps[i].x1).max().unwrap();
        let ty0 = tc.iter().map(|&i| comps[i].y0).min().unwrap();
        let ty1 = tc.iter().map(|&i| comps[i].y1).max().unwrap();
        text_box = Some((tx0, ty0, tx1, ty1));
        let px: Vec<(u32, u32)> = tc.iter().flat_map(|&i| comps[i].px.iter().copied()).collect();
        res.fg = Some(median_color(rgb, w, &px));
    }

    // ---- Icon: kompakter Cluster ausserhalb der Textkette ------------------
    let mut icon: Option<C> = None;
    if image_hint {
        let in_chain: Vec<bool> = {
            let mut v = vec![false; comps.len()];
            if let Some(tc) = &text_chain {
                for &i in tc {
                    v[i] = true;
                }
            }
            v
        };
        let mut cand_idx: Vec<usize> = (0..comps.len()).filter(|&i| !in_chain[i]).collect();
        for &s in &stripped {
            if !cand_idx.contains(&s) {
                cand_idx.push(s);
            }
        }
        let cands: Vec<C> = cand_idx.iter().map(|&i| comps[i].clone()).collect();
        let clusters = cluster_boxes(cands, 4);
        let score = |c: &C| {
            let aspect = c.w().max(c.h()) as f64 / c.w().min(c.h()).max(1) as f64;
            (c.n as f64).sqrt() * c.dens() / aspect
        };
        let compact =
            |c: &C| c.w().max(c.h()) as f64 / c.w().min(c.h()).max(1) as f64 <= 2.2;
        let nmin = ((0.15 * hf) as usize).max(8);
        let mut best: Option<usize> = None;
        for (i, c) in clusters.iter().enumerate() {
            if c.n >= nmin && score(c) >= 0.6 && compact(c) {
                if best.map(|b| score(c) > score(&clusters[b])).unwrap_or(true) {
                    best = Some(i);
                }
            }
        }
        if let Some(bi) = best {
            icon = Some(clusters[bi].clone());
            // Rettung: Fett-Text, der als Icon-Kandidat gelandet ist
            if text_chain.is_none() && caption_hint {
                let mut tb: Option<usize> = None;
                for (i, c) in clusters.iter().enumerate() {
                    if i != bi
                        && c.w() as f64 >= 2.2 * c.h() as f64
                        && c.w() >= 12
                        && c.h() >= 4
                        && c.n as f64 <= 0.75 * c.w() as f64 * c.h() as f64
                    {
                        if tb.map(|t| c.n > clusters[t].n).unwrap_or(true) {
                            tb = Some(i);
                        }
                    }
                }
                if let Some(ti) = tb {
                    let t = &clusters[ti];
                    res.cap_present = true;
                    text_box = Some((t.x0, t.y0, t.x1, t.y1));
                    res.fg = Some(median_color(rgb, w, &t.px));
                    text_chain = Some(Vec::new()); // Marker: Text vorhanden
                }
            }
        }
    }
    if let Some(ic) = &icon {
        res.img_present = true;
        let icx = (ic.x0 + ic.x1) as f64 / 2.0;
        let icy = (ic.y0 + ic.y1) as f64 / 2.0;
        res.img_h = h_class(icx, 0.0, wf);
        res.img_v = v_class(icy, 0.0, hf);
    }
    let _ = text_chain; // (nur noch text_box wird gebraucht)

    // ---- Caption-Position: margin-basiert in der Restflaeche ---------------
    if let Some((tx0, ty0, tx1, ty1)) = text_box {
        let tcy = (ty0 + ty1) as f64 / 2.0;
        let mut ax0 = 0.0f64;
        let mut ax1 = wf;
        if let Some(ic) = &icon {
            if res.img_v == P_CENTER {
                if res.img_h == P_LEFT {
                    ax0 = (ic.x1 + 1) as f64;
                } else if res.img_h == P_RIGHT {
                    ax1 = (ic.x0 - 1) as f64;
                }
            }
        }
        let aw = (ax1 - ax0).max(1.0);
        let m0 = (tx0 as f64 - ax0) / aw;
        let m1 = (ax1 - tx1 as f64) / aw;
        res.cap_h = if (m0 - m1).abs() <= 0.18 {
            P_CENTER
        } else if m0 < m1 {
            P_LEFT
        } else {
            P_RIGHT
        };
        res.cap_v = v_class(tcy, 0.0, hf);
    }
    res
}

fn rgb_i32(c: [f64; 3]) -> i32 {
    let r = c[0].round().clamp(0.0, 255.0) as i32;
    let g = c[1].round().clamp(0.0, 255.0) as i32;
    let b = c[2].round().clamp(0.0, 255.0) as i32;
    (r << 16) | (g << 8) | b
}

fn caption_slice<'a>(caption: *const u8) -> Option<&'a [u8]> {
    if caption.is_null() {
        return None;
    }
    let mut len = 0usize;
    unsafe {
        while len < 4096 && *caption.add(len) != 0 {
            len += 1;
        }
        Some(std::slice::from_raw_parts(caption, len))
    }
}

// ---------------------------------- FFI ------------------------------------

/// v3: Analysiert einen Button-Ausschnitt (BMP-Bytes, 24/32 Bit).
/// caption: Caption-TEXT als nullterminierter ANSI-String (NULL/leer = die
/// Caption-Property ist leer). Ob die Caption angezeigt wird, wird ERKANNT.
/// image_hint: 1 = Bild-Property belegt, 0 = kein Bild.
/// Rueckgabe wie WDLayoutAnalyzeBMP gepackt:
///   bits 0-1 capH, 2-3 capV, 4 captionSichtbar, 5-6 imgH, 7-8 imgV, 9 hasImage
///   (H: 1=links 2=mitte 3=rechts; V: 1=oben 2=mitte 3=unten), negativ=Fehler.
/// WLanguage: nP = CallDLL32(sDLL,"WDLayoutAnalyzeBMP2", bufBMP, Length(bufBMP), sCaption, nImg)
#[no_mangle]
pub extern "C" fn WDLayoutAnalyzeBMP2(
    data: *const u8,
    len: i32,
    caption: *const u8,
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
    let r = analyze3(w, h, &rgb, caption_slice(caption), image_hint != 0);
    pack_result(r.cap_present, r.cap_h, r.cap_v, r.img_present, r.img_h, r.img_v)
}

/// v3 mit Rohpixeln. fmt: 0=RGB, 1=BGR (3 B/px), 2=BGRA, 3=RGBA. stride=0 => dicht.
#[no_mangle]
pub extern "C" fn WDLayoutAnalyzeRaw2(
    data: *const u8,
    width: i32,
    height: i32,
    stride: i32,
    fmt: i32,
    caption: *const u8,
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
    let r = analyze3(w, h, &rgb, caption_slice(caption), image_hint != 0);
    pack_result(r.cap_present, r.cap_h, r.cap_v, r.img_present, r.img_h, r.img_v)
}

/// Hintergrund-(Fuell-)Farbe des Buttons als 0xRRGGBB (negativ = Fehler).
#[no_mangle]
pub extern "C" fn WDLayoutBgColor(data: *const u8, len: i32) -> i32 {
    if data.is_null() || len < 54 {
        return LAYOUT_ERR_ARGS;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    let (w, h, rgb) = match parse_bmp(slice) {
        Some(v) => v,
        None => return LAYOUT_ERR_BMP,
    };
    let r = analyze3(w, h, &rgb, None, false);
    rgb_i32(r.bg)
}

/// 1 = dunkler Button-Hintergrund, 0 = heller (negativ = Fehler).
#[no_mangle]
pub extern "C" fn WDLayoutIsDark(data: *const u8, len: i32) -> i32 {
    if data.is_null() || len < 54 {
        return LAYOUT_ERR_ARGS;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len as usize) };
    let (w, h, rgb) = match parse_bmp(slice) {
        Some(v) => v,
        None => return LAYOUT_ERR_BMP,
    };
    let r = analyze3(w, h, &rgb, None, false);
    if r.dark {
        1
    } else {
        0
    }
}

/// Schriftfarbe der erkannten Caption als 0xRRGGBB.
/// -3 = keine Caption erkannt (dann gibt es auch keine Schriftfarbe).
#[no_mangle]
pub extern "C" fn WDLayoutFgColor(
    data: *const u8,
    len: i32,
    caption: *const u8,
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
    let r = analyze3(w, h, &rgb, caption_slice(caption), image_hint != 0);
    match r.fg {
        Some(c) => rgb_i32(c),
        None => LAYOUT_ERR_NO_TEXT,
    }
}

// --------------------------------- Tests -----------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// Vergleicht den Rust-Port Feld fuer Feld mit den Soll-Werten des
    /// Python-Prototyps (expected.txt, erzeugt aus layout_v3_prototyp.py
    /// ueber alle 207 gelabelten Buttons der Themes Eleven/Cobalt/Ankaa).
    #[test]
    fn port_identisch_zum_prototyp() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples_layout");
        let exp = fs::read_to_string(dir.join("expected.txt")).expect("expected.txt fehlt");
        let caption = b"XX XX\0"; // Testfenster-Captions haben 4 Glyphen
        let mut checked = 0;
        for line in exp.lines() {
            let f: Vec<&str> = line.trim().split(';').collect();
            if f.len() < 9 {
                continue;
            }
            let data = fs::read(dir.join(f[0])).unwrap_or_else(|_| panic!("fehlt: {}", f[0]));
            let (w, h, rgb) = parse_bmp(&data).expect("BMP");
            let r = analyze3(w, h, &rgb, Some(&caption[..5]), true);
            let got = (
                r.cap_present as i32,
                r.cap_h,
                r.cap_v,
                r.img_present as i32,
                r.img_h,
                r.img_v,
                r.dark as i32,
            );
            let want = (
                f[1].parse::<i32>().unwrap(),
                f[2].parse::<i32>().unwrap(),
                f[3].parse::<i32>().unwrap(),
                f[4].parse::<i32>().unwrap(),
                f[5].parse::<i32>().unwrap(),
                f[6].parse::<i32>().unwrap(),
                f[7].parse::<i32>().unwrap(),
            );
            assert_eq!(got, want, "Abweichung bei {}", f[0]);
            // Hintergrundfarbe (gerundet) vergleichen, Toleranz 1/Kanal
            let bgw: Vec<i32> = f[8].split(',').map(|v| v.parse().unwrap()).collect();
            for ch in 0..3 {
                let g = r.bg[ch].round() as i32;
                assert!(
                    (g - bgw[ch]).abs() <= 1,
                    "bg-Kanal {} bei {}: {} vs {}",
                    ch,
                    f[0],
                    g,
                    bgw[ch]
                );
            }
            checked += 1;
        }
        assert_eq!(checked, 207, "erwartet 207 Testfaelle, geprueft: {}", checked);
    }

    /// Navigations-Buttons: Caption ist "<", ">>" usw. (1-2 Glyphen), KEIN
    /// Bild zugewiesen -> alles Gerenderte muss als Caption erkannt werden.
    /// (Die NC_IC-Samples liefern genau so einen Einzel-Glyph-Render.)
    #[test]
    fn nav_buttons_einzelglyph_caption() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples_layout");
        for f in [
            "Eleven_BTN_PLUS_NC_IC_1.bmp",
            "Eleven_BTN_ARROW_DOWN_NC_IC_3.bmp",
            "Ankaa_BTN_QUESTION_NC_IC_1.bmp",
            "Cobalt_BTN_PLUS_NC_IC_2.bmp",
        ] {
            let d = fs::read(dir.join(f)).unwrap();
            // imageHint=0 + Caption "<" -> Caption erkannt, kein Icon
            let p = WDLayoutAnalyzeBMP2(d.as_ptr(), d.len() as i32, b"<\0".as_ptr(), 0);
            assert!(p >= 0, "{}: Fehler {}", f, p);
            assert_eq!((p >> 4) & 1, 1, "{}: Caption muss erkannt werden", f);
            assert_eq!((p >> 9) & 1, 0, "{}: kein Icon erwartet", f);
        }
    }

    #[test]
    fn ffi_smoke() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples_layout");
        let d = fs::read(dir.join("Eleven_BTN_ARROW_DOWN_NC_IC_1.bmp")).unwrap();
        // Icon-only-Button MIT caption-Text (Button3-Fall): Caption muss AUS sein
        let p = WDLayoutAnalyzeBMP2(d.as_ptr(), d.len() as i32, b"Egal\0".as_ptr(), 1);
        assert!(p >= 0);
        assert_eq!((p >> 4) & 1, 0, "Caption muss als nicht angezeigt erkannt werden");
        assert_eq!((p >> 9) & 1, 1, "Icon muss erkannt werden");
        // Farb-Exporte
        assert!(WDLayoutBgColor(d.as_ptr(), d.len() as i32) >= 0);
        assert_eq!(WDLayoutIsDark(d.as_ptr(), d.len() as i32), 0);
        assert_eq!(
            WDLayoutFgColor(d.as_ptr(), d.len() as i32, b"Egal\0".as_ptr(), 1),
            LAYOUT_ERR_NO_TEXT
        );
        // Fehlerfaelle
        assert_eq!(WDLayoutAnalyzeBMP2(std::ptr::null(), 0, std::ptr::null(), 0), LAYOUT_ERR_ARGS);
    }
}
