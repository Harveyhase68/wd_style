# -*- coding: utf-8 -*-
"""
layout_v3 — WinDev-Button-Layout-Detektor (Prototyp fuer wd_style/layout.rs v3)

Neu gegenueber v2:
 - Caption-PRAESENZ wird ERKANNT (nicht mehr blind dem Hint geglaubt):
   Hint bedeutet nur noch "Caption-Text ist nicht leer" -> Anzeige kann trotzdem
   aus sein (Button3-Fall!).
 - Text = Kette buchstabenartiger Komponenten (aehnliche Hoehe, Baseline,
   horizontale Folge). Icon = kompakte Komponente ausserhalb der Kette.
 - Mehrfach-Hintergrund (Button-Fuellung + Fenster-Ecken + Rahmen) via
   dominante Farb-Cluster.
 - Zusatz-Outputs: Hintergrundfarbe (Fuellung), Schriftfarbe, hell/dunkel.

Nur numpy-Basisoperationen, bewusst 1:1 nach Rust portierbar.
"""
import numpy as np


def dominant_colors(rgb, min_share=0.06):
    """Dominante Farben (quantisiert 32er-Schritte), Anteil >= min_share."""
    q = (rgb // 32).astype(np.int32)
    key = q[:, :, 0] * 64 + q[:, :, 1] * 8 + q[:, :, 2]
    vals, counts = np.unique(key.ravel(), return_counts=True)
    order = np.argsort(-counts)
    total = key.size
    out = []
    for i in order:
        if counts[i] / total < min_share:
            break
        m = key == vals[i]
        mean = rgb[m].mean(axis=0)
        out.append((mean, counts[i] / total, m))
    return out


def ink_mask(rgb):
    """Tinte = Pixel weit weg von allen dominanten Hintergrundfarben.

    Bekannte Grenze (naechster Ausbauschritt, s. Plan): Icon in einer Farbe,
    die gleichzeitig dominante Hintergrundfarbe ist (weisses Icon + weisse
    Fensterecken, Cobalt-NC-Buttons) -> Loesung: Konnektivitaet pro
    Farbregion (randverbundene Regionen = Hintergrund, Inseln = Tinte)."""
    doms = dominant_colors(rgb)
    if not doms:
        return np.zeros(rgb.shape[:2], bool), np.array([255, 255, 255.0])
    dist = None
    for mean, _, _ in doms:
        d = np.abs(rgb.astype(np.float64) - mean).sum(axis=2)
        dist = d if dist is None else np.minimum(dist, d)
    thr = max(90.0, dist.max() * 0.35)
    mask = dist > thr
    fill = doms[0][0]  # haeufigste Farbe = Button-Fuellung
    return mask, fill


def ink_mask_islands(rgb):
    """Fallback, wenn die Dominanzfarben-Maske nichts findet: Tinte = Regionen,
    die einer dominanten Farbe angehoeren, aber den BILDRAND NICHT beruehren
    (Inseln). Loest 'weisses Icon auf hellem Fill neben weissen Fensterecken':
    die Ecken beruehren den Rand (Hintergrund), das Icon nicht (Tinte).
    Kein lokaler Flood-Fill -> sickert nicht durch Anti-Aliasing."""
    H, W, _ = rgb.shape
    doms = dominant_colors(rgb)
    if not doms:
        return np.zeros((H, W), bool)
    T = 60.0
    bg = np.zeros((H, W), bool)
    for mean, _, _ in doms:
        d = np.abs(rgb.astype(np.float64) - mean).sum(axis=2)
        mask_d = d <= T
        for c in components(mask_d):
            if c['x0'] == 0 or c['y0'] == 0 or c['x1'] == W - 1 or c['y1'] == H - 1:
                for (y, x) in c['px']:
                    bg[y, x] = True
    # Tinte = alles, was zu keiner randberuehrenden Dominanzfarb-Region gehoert
    return ~bg


def components(mask):
    """8er-Zusammenhangskomponenten, iterativ (Rust-portierbar)."""
    H, W = mask.shape
    lab = np.full((H, W), -1, np.int32)
    comps = []
    for y0 in range(H):
        for x0 in range(W):
            if not mask[y0, x0] or lab[y0, x0] >= 0:
                continue
            idx = len(comps)
            stack = [(y0, x0)]
            lab[y0, x0] = idx
            px = []
            while stack:
                y, x = stack.pop()
                px.append((y, x))
                for dy in (-1, 0, 1):
                    for dx in (-1, 0, 1):
                        ny, nx = y + dy, x + dx
                        if 0 <= ny < H and 0 <= nx < W and mask[ny, nx] and lab[ny, nx] < 0:
                            lab[ny, nx] = idx
                            stack.append((ny, nx))
            ys = [p[0] for p in px]; xs = [p[1] for p in px]
            comps.append({
                'n': len(px),
                'x0': min(xs), 'x1': max(xs), 'y0': min(ys), 'y1': max(ys),
                'px': px,
            })
    return comps


def expected_glyphs(caption):
    """Erwartete Komponentenzahl eines gerenderten Texts (Punkte zaehlen extra)."""
    n = 0
    for ch in caption:
        if ch.isspace():
            continue
        n += 1
        if ch in 'ijäöüÄÖÜ!?:;=':
            n += 1
    return n


def cluster_boxes(cands, gap=4):
    """Nicht-Ketten-Komponenten zu Icon-Clustern verschmelzen (BBox-Abstand <= gap)."""
    clusters = [dict(c, members=[c]) for c in cands]
    changed = True
    while changed:
        changed = False
        for i in range(len(clusters)):
            for j in range(i + 1, len(clusters)):
                a, b2 = clusters[i], clusters[j]
                if (a['x0'] - gap <= b2['x1'] and b2['x0'] - gap <= a['x1'] and
                        a['y0'] - gap <= b2['y1'] and b2['y0'] - gap <= a['y1']):
                    a['x0'] = min(a['x0'], b2['x0']); a['x1'] = max(a['x1'], b2['x1'])
                    a['y0'] = min(a['y0'], b2['y0']); a['y1'] = max(a['y1'], b2['y1'])
                    a['n'] += b2['n']; a['px'] = a['px'] + b2['px']
                    a['members'] += b2['members']
                    del clusters[j]
                    changed = True
                    break
            if changed:
                break
    return clusters


def analyze(rgb, caption_hint=1, image_hint=1, caption_text=None):
    """rgb: HxWx3 uint8. Hints: caption_hint=1 wenn Caption-TEXT nicht leer
    (Anzeige unbekannt!), image_hint=1 wenn ein Bild zugewiesen ist.
    Rueckgabe dict mit capPresent, capH/'', capV, imgPresent, imgH, imgV,
    bg, fg, dark, Debug-Boxen."""
    H, W, _ = rgb.shape
    mask, fill = ink_mask(rgb)

    # Rahmen/Rand ausblenden: 2px umlaufend
    b = 2
    mask2 = mask.copy()
    mask2[:b, :] = False; mask2[-b:, :] = False
    mask2[:, :b] = False; mask2[:, -b:] = False

    def is_line(c):
        w = c['x1']-c['x0']+1; h = c['y1']-c['y0']+1
        if h <= 3 and w >= 0.5*W: return True
        if w <= 3 and h >= 0.5*H: return True
        return False

    def build_comps(m):
        cs = [c for c in components(m) if c['n'] >= 4]
        # Rahmenreste: Komponenten, die fast die ganze Breite UND Hoehe umspannen
        cs = [c for c in cs if not ((c['x1']-c['x0']) > 0.92*W and (c['y1']-c['y0']) > 0.92*H)]
        # Rahmen-/Glow-LINIEN: sehr flach+breit oder sehr schmal+hoch am Rand
        return [c for c in cs if not is_line(c)]

    comps = build_comps(mask2)
    if not comps:
        # Fallback: Insel-Segmentierung (Icon-Farbe == dominante Hintergrundfarbe)
        mask3 = ink_mask_islands(rgb)
        mask3[:b, :] = False; mask3[-b:, :] = False
        mask3[:, :b] = False; mask3[:, -b:] = False
        comps = build_comps(mask3)

    res = {'bg': fill.tolist(), 'dark': float((fill*[.299,.587,.114]).sum()) < 128,
           'capPresent': 0, 'capH': 0, 'capV': 0,
           'imgPresent': 0, 'imgH': 0, 'imgV': 0,
           'fg': None, 'textBox': None, 'iconBox': None}
    if not comps:
        return res

    def h_of(c): return c['y1'] - c['y0'] + 1
    def w_of(c): return c['x1'] - c['x0'] + 1

    # ---- Textketten-Suche: buchstabenartige Komponenten gruppieren --------
    # Buchstabe: Hoehe 5..0.9H, Breite <= 2.2*Hoehe (schmal), Flaeche klein
    letters = [c for c in comps if 4 <= h_of(c) <= 0.95*H and w_of(c) <= 2.4*h_of(c)]
    # Ketten: sortiert nach x; gleiche Baseline (y-Ueberlappung >=50%),
    # Hoehenverhaeltnis <= 1.9, Luecke <= 1.2 * max Hoehe
    chains = []
    for c in sorted(letters, key=lambda c: c['x0']):
        placed = False
        for ch in chains:
            last = ch[-1]
            ovl = min(c['y1'], last['y1']) - max(c['y0'], last['y0']) + 1
            hmin = min(h_of(c), h_of(last))
            hmax = max(h_of(c), h_of(last))
            gap = c['x0'] - last['x1']
            if ovl >= 0.5*hmin and hmax <= 1.9*hmin and -2 <= gap <= 1.3*hmax:
                ch.append(c); placed = True; break
        if not placed:
            chains.append([c])
    chains.sort(key=lambda ch: -len(ch))

    text_chain = None
    stripped = []
    if caption_hint:
        # Weg A: Kette buchstabenartiger Komponenten (>= 3 Glieder).
        # Weg B: verschmolzener Text = breiter flacher Blob (fette/kleine
        #        Schrift klebt zusammen, z.B. Ankaa-Theme!).
        exp0 = expected_glyphs(caption_text) if caption_text else None
        blob = None
        for c in comps:
            w, h = w_of(c), h_of(c)
            dens = c['n'] / (w*h)
            # Text-Blob: breit, flach, LOECHRIG (Linien/Balken haben Dichte ~1)
            if w >= 2.2*h and w >= 12 and 4 <= h <= 0.7*H and dens <= 0.75:
                if blob is None or c['n'] > blob['n']:
                    blob = c
        if chains and len(chains[0]) >= 3:
            text_chain = list(chains[0])
        # kurze Kette akzeptieren, wenn Glyphenzahl passt oder sonst nichts da ist
        elif chains and len(chains[0]) == 2:
            rest = [c for c in comps if c not in chains[0]]
            # wortartig: Kette INSGESAMT deutlich breiter als hoch
            # (verschmolzene Woerter fetter kleiner Schrift, z.B. "CL"+"IR")
            cw2 = max(c['x1'] for c in chains[0]) - min(c['x0'] for c in chains[0]) + 1
            ch2 = max(h_of(c) for c in chains[0])
            wordy = (cw2 >= 2.2*ch2 and cw2 >= 16 and
                     all(c['n'] <= 0.85*w_of(c)*h_of(c) for c in chains[0]))
            if not rest or wordy or (exp0 is not None and exp0 <= 3):
                text_chain = list(chains[0])
        # Blob gewinnt, wenn keine (oder eine kleinere) Kette gefunden wurde
        if blob is not None:
            chain_w = 0
            if text_chain is not None:
                chain_w = max(c['x1'] for c in text_chain) - min(c['x0'] for c in text_chain)
            if text_chain is None or (blob not in text_chain and w_of(blob) > 1.2*chain_w):
                text_chain = [blob]

    # Nav-Buttons: Caption-Property gesetzt, Bild-Property LEER und keine
    # Textkette gefunden (1-2-Glyphen-Captions wie "<", ">>"): alles = Caption.
    # Style-Icons trotz leerer Bild-Property erkennt die Icon-Suche unten.
    if caption_hint and not image_hint and text_chain is None and comps:
        text_chain = list(comps)

    # Juxtaposed-Icon: klebt an der Textkette. Wenn die Caption bekannt ist
    # und die Kette MEHR Glieder hat als erwartet, Hoehen-Ausreisser am
    # Kettenende abspalten -> Icon-Kandidat.
    if text_chain is not None and caption_text is not None and image_hint:
        exp = expected_glyphs(caption_text)
        heights = sorted(h_of(c) for c in text_chain)
        med = heights[len(heights)//2]
        while len(text_chain) > max(1, exp):
            first, last = text_chain[0], text_chain[-1]
            fdev = abs(h_of(first) - med)
            ldev = abs(h_of(last) - med)
            cand, side = (first, 0) if fdev >= ldev else (last, -1)
            if abs(h_of(cand) - med) > max(2, 0.2 * med):
                stripped.append(cand); del text_chain[side]; continue
            # Fallback 2: Luecken-Ausreisser am Kettenende
            if len(text_chain) >= 3:
                gaps = [text_chain[i+1]['x0'] - text_chain[i]['x1']
                        for i in range(len(text_chain)-1)]
                inner = sorted(gaps[1:-1]) or sorted(gaps)
                medgap = inner[len(inner)//2]
                if gaps[0] >= 2 + 1.6*medgap and gaps[0] >= gaps[-1]:
                    stripped.append(text_chain[0]); del text_chain[0]; continue
                if gaps[-1] >= 2 + 1.6*medgap:
                    stripped.append(text_chain[-1]); del text_chain[-1]; continue
            # Fallback 3: quadratisches Endglied (Icons sind ~quadratisch,
            # greift nur bei Ketten-UEBERlaenge, echte Captions haben len==exp)
            fsq = abs(w_of(first) - h_of(first)) <= max(2, 0.25*h_of(first))
            lsq = abs(w_of(last) - h_of(last)) <= max(2, 0.25*h_of(last))
            if fsq and not lsq:
                stripped.append(text_chain[0]); del text_chain[0]; continue
            if lsq and not fsq:
                stripped.append(text_chain[-1]); del text_chain[-1]; continue
            break  # kein klarer Kandidat -> nicht abspalten
        if len(text_chain) < 1:
            text_chain = None

    if text_chain is not None:
        res['capPresent'] = 1
        tx0 = min(c['x0'] for c in text_chain); tx1 = max(c['x1'] for c in text_chain)
        ty0 = min(c['y0'] for c in text_chain); ty1 = max(c['y1'] for c in text_chain)
        res['textBox'] = [tx0, ty0, tx1, ty1]
        # Schriftfarbe = Median der Kettenpixel
        pix = np.array([rgb[y, x] for c in text_chain for (y, x) in c['px']])
        res['fg'] = np.median(pix, axis=0).tolist()

    # ---- Icon: kompakte Komponente/Cluster ausserhalb der Textkette --------
    # Laeuft auch bei image_hint=0, wenn eine Textkette existiert: Gabarit-
    # Styles koennen Icons rendern, OHNE dass die Bild-Property belegt ist!
    icon = None
    if image_hint or text_chain is not None:
        in_chain = set()
        if text_chain is not None:
            for c in text_chain:
                in_chain.add(id(c))
        cands = [c for c in comps if id(c) not in in_chain]
        # abgespaltene Juxtaposed-Glieder sind Icon-Kandidaten
        for c in stripped:
            if id(c) not in in_chain:
                pass
        cands = cands + [c for c in stripped if c not in cands]
        # mehrteilige Icons (?-Punkt, Pfeil-Unterstrich) verschmelzen
        clusters = cluster_boxes(cands, gap=4) if cands else []
        def score(c):
            w, h = w_of(c), h_of(c)
            aspect = max(w, h) / max(1, min(w, h))
            density = c['n'] / (w * h)
            import math
            return math.sqrt(c['n']) * density / aspect
        # Icon muss KOMPAKT sein (aspect <= 2.2) und Mindestflaeche haben
        # (relativ zur Buttonhoehe, damit kleine Buttons kleine Icons erlauben)
        nmin = max(8, int(0.15 * H))
        def compact(c):
            w, h = w_of(c), h_of(c)
            return max(w, h) / max(1, min(w, h)) <= 2.2
        ok = [c for c in clusters if c['n'] >= nmin and score(c) >= 0.6 and compact(c)]
        if ok:
            icon = max(ok, key=score)
            if not image_hint:
                # Style-Icon ohne Bild-Property: nur akzeptieren, wenn es
                # (a) deutlich von der Caption getrennt ist ODER (b) mehr
                # Komponenten existieren, als die Caption Glyphen hat.
                accept = False
                if res['textBox'] is not None:
                    tx0b, ty0b, tx1b, ty1b = res['textBox']
                    th2 = ty1b - ty0b + 1
                    if icon['x0'] > tx1b:
                        hgap = icon['x0'] - tx1b
                    elif tx0b > icon['x1']:
                        hgap = tx0b - icon['x1']
                    else:
                        hgap = 0
                    separated = hgap >= 1.5 * th2
                    exp2 = expected_glyphs(caption_text) if caption_text else 0
                    extra = len(comps) > exp2
                    accept = separated or extra
                if not accept:
                    icon = None
        # Rettung fuer verschmolzenen Fett-Text, der faelschlich als Icon-
        # Kandidat gelandet ist: wenn kein Text erkannt wurde, aber caption_hint
        # gesetzt ist und es neben dem kompakten Icon einen BREITEN flachen
        # Cluster gibt -> der breite ist der Text.
        if (text_chain is None and caption_hint and icon is not None):
            wide = [c for c in clusters if c is not icon and w_of(c) >= 2.2*h_of(c)
                    and w_of(c) >= 12 and h_of(c) >= 4
                    and c['n'] <= 0.75*w_of(c)*h_of(c)]
            if wide:
                tblob = max(wide, key=lambda c: c['n'])
                text_chain = [tblob]
                res['capPresent'] = 1
                tx0, ty0, tx1, ty1 = tblob['x0'], tblob['y0'], tblob['x1'], tblob['y1']
                res['textBox'] = [tx0, ty0, tx1, ty1]
                pix = np.array([rgb[y, x] for (y, x) in tblob['px']])
                res['fg'] = np.median(pix, axis=0).tolist()
    # ---- Rettungs-Pass: Icon mit schwaechlicherem Kontrast als der Text ----
    # Der adaptive Schwellwert oben skaliert mit dist.max() (dominiert vom
    # dunkelsten Element im Bild, meist der Text). Ein GRAUES Icon neben
    # SCHWARZEM Text (z.B. "New [] "-Buttons) faellt dann unter den Tisch,
    # weil seine Distanz zur Hintergrundfarbe kleiner ist als die dynamisch
    # angehobene Schwelle. Fix: bei fehlendem Icon zusaetzlich mit einer
    # FESTEN, niedrigeren Schwelle suchen — aber NUR ausserhalb der Caption-
    # Box (inkl. Rand), damit die bestehende Text-/Icon-Trennung fuer alle
    # anderen Faelle unangetastet bleibt (0 Regressionen im 207er-Testset,
    # 0 Geister-Icons an reinen Text-Buttons ohne jedes Bild verifiziert).
    if icon is None and res['textBox'] is not None:
        tx0r, ty0r, tx1r, ty1r = res['textBox']
        dist_r = None
        for mean, _, _ in dominant_colors(rgb):
            d = np.abs(rgb.astype(np.float64) - mean).sum(axis=2)
            dist_r = d if dist_r is None else np.minimum(dist_r, d)
        mask_r = dist_r > 90.0
        mask_r[:b, :] = False; mask_r[-b:, :] = False
        mask_r[:, :b] = False; mask_r[:, -b:] = False
        margin = 2
        mask_r[max(0, ty0r-margin):ty1r+margin+1, max(0, tx0r-margin):tx1r+margin+1] = False
        rcands = [c for c in components(mask_r) if c['n'] >= 4]
        rcands = [c for c in rcands if not is_line(c)]
        rclusters = cluster_boxes(rcands, gap=4) if rcands else []
        nmin_r = max(8, int(0.15 * H))
        def rscore(c):
            w, h = w_of(c), h_of(c)
            aspect = max(w, h) / max(1, min(w, h))
            return (c['n'] ** 0.5) * (c['n']/(w*h)) / aspect
        def rcompact(c):
            # Grosszuegiger als das normale Icon-Kompaktheitsmass: schmale
            # Balken-Icons (Minus, Trennstriche) sind erlaubt, solange die
            # Flaeche + Dichte (rscore) stimmt; extreme Linien filtert is_line
            # bereits vorher weg (h<=3 & w>=0.5*W).
            w, h = w_of(c), h_of(c)
            return max(w, h) / max(1, min(w, h)) <= 4.5
        rok = [c for c in rclusters if c['n'] >= nmin_r and rscore(c) >= 0.6 and rcompact(c)]
        if rok:
            icon = max(rok, key=rscore)

    if icon is not None:
        res['imgPresent'] = 1
        res['iconBox'] = [icon['x0'], icon['y0'], icon['x1'], icon['y1']]

    # ---- Positionen --------------------------------------------------------
    def h_class(cx, x0, x1):
        rel = (cx - x0) / max(1.0, (x1 - x0))
        return 1 if rel < 0.38 else (3 if rel > 0.62 else 2)
    def v_class(cy, y0, y1):
        rel = (cy - y0) / max(1.0, (y1 - y0))
        return 1 if rel < 0.35 else (3 if rel > 0.65 else 2)

    if icon is not None:
        icx = (icon['x0'] + icon['x1']) / 2; icy = (icon['y0'] + icon['y1']) / 2
        res['imgH'] = h_class(icx, 0, W); res['imgV'] = v_class(icy, 0, H)
    if text_chain is not None:
        tcy = (ty0 + ty1) / 2
        # Caption RELATIV zur Restflaeche (Button minus Icon-Seite) bewerten,
        # wenn Icon links/rechts sitzt (Kalibrierung aus v1!)
        ax0, ax1 = 0, W
        if icon is not None and res['imgV'] == 2:
            if res['imgH'] == 1: ax0 = icon['x1'] + 1
            elif res['imgH'] == 3: ax1 = icon['x0'] - 1
        # margin-basiert: welche Seite hat den kleineren Rand?
        aw = max(1.0, ax1 - ax0)
        m0 = (tx0 - ax0) / aw
        m1 = (ax1 - tx1) / aw
        if abs(m0 - m1) <= 0.18:
            res['capH'] = 2
        else:
            res['capH'] = 1 if m0 < m1 else 3
        res['capV'] = v_class(tcy, 0, H)
    return res


if __name__ == '__main__':
    import sys, glob, os
    from PIL import Image
    for f in sys.argv[1:]:
        rgb = np.asarray(Image.open(f).convert('RGB'))
        r = analyze(rgb)
        print(os.path.basename(f), r)
