# Befund: Titel-Hintergrundfarbe im WinDev ..Style (Tabellen-Style)

Stand: 2026-08-09. Datensatz: `WIN_TEST3.TABLE_TEST3_<RRGGBB>.bin`
(nur die Titel-Hintergrundfarbe variiert, Border-Image absichtlich gelöscht).

## Was gesichert ist

1. **Kompletter Stream ≈ 10127 Bytes.** Der Header deklariert nur den ersten
   Sub-Block (5520 B); der LZHUF-Stream läuft darüber hinaus weiter und
   enthält weitere Sub-Style-Blöcke. `WDStyleUncompress()` (Rust) und
   `wd_table_color_analyse.py decode` liefern denselben vollen Stream.

2. **Farbe beginnt bei Offset 4381.** Gemeinsamer Präfix über alle 15
   Farbvarianten = exakt 4381 Bytes. Anker davor: `… d4 ac 7e 76 d7`.

3. **Farbe steckt in einer wiederkehrenden 18-Byte-Record-Tabelle**
   (Farbe je Zustand: normal / hover / selektiert …). Schwarz zeigt den
   Record `c0 00 be 00 c9 ff e9 7f c2 92 3c 46 e9 79 00 00 00 00` ×3.

4. **Kein Klartext-RGB.** Weder `RR GG BB` noch `BB GG RR` noch der
   WinDev-COLORREF taucht auf. Die Farbe ist ein **verwürfeltes
   Property-Token** — dieselbe Obfuskation wie die Caption-Positionswerte
   (`TOP = 8F 97 5F D9` usw.). Beispiel Titelfarbe:
   - RGB `000000` → `c0 00 be 00 c9 …`
   - RGB `010000` → `a7 f9 58 f5 00 …`

## Was ausgeschlossen ist

- **Reines XOR (mit/ohne Salt):** ausgeschlossen. Eine 1-Byte-Klartext-
  änderung würde bei XOR nur 1 Byte im Ergebnis ändern. Tatsächlich:
  `010000` ändert 63 Bytes, `020000` ~4550 Bytes (nichtlineare Kaskade).
- **Zweiter eigenständiger LZHUF/zlib/gzip-Stream:** ausgeschlossen. Keine
  Magic-Bytes (`78 9c`, `1f 8b`), verschachteltes LZHUF-Decode an den
  Kandidaten-Offsets liefert keine Struktur (Entropie bleibt ~5,1).

## Deutung der Nichtlinearität

`010000` bleibt lokal (63 B), fast alle anderen Farben kaskadieren über
den halben Stream. Ursache: die verwürfelten Record-Bytes landen im
LZHUF-Ringpuffer und werden von späteren Matches referenziert → kleiner
logischer Change, große Byte-Kaskade. Der **stabile Token-Schwanz**
`ff e9 7f c2 92 3c 46 e9 79` existiert nur bei den lokalen Fällen; sobald
der Wert länger wird, verwürfelt der ganze Record → **kein fixer Anker**
quer über alle Farben.

## Konsequenz für das Ziel

- **Lookup-Tabelle** (wie bei den 12 Caption-Positionen) ist für eine
  24-Bit-Farbe (16 Mio Werte) **nicht praktikabel**.
- Zum Auslesen der Farbe muss die **WinDev-Scramble-Funktion für
  Property-Werte invertiert** werden. Das ist derselbe Schritt, der auch
  die Positionswerte lesbar machen würde (die sind bisher nur per
  Lookup gemappt, nicht entschlüsselt).

## Empfohlene nächste Schritte

1. **Scramble reversen** — bevorzugt aus dem WinDev-Framework (die
   Serialisierungsfunktion in `wd290vm.dll` / `wd290obj.dll`), analog zum
   Vorgehen, mit dem der LZHUF geknackt wurde.
2. **Falls empirisch:** dichten Ein-Kanal-Sweep exportieren, der lokal
   bleibt (wie `010000`), um den 4–5-Byte-Scramble per Korrelation zu
   reversen. Aktuell liegt genau **ein** sauberes Paar vor
   (schwarz ↔ 010000).
3. **Schritt 2 (Button-Image-Name):** ein Sample MIT gesetztem Image
   exportieren — der Name ist vermutlich Klartext-ASCII im Stream
   (im aktuellen bild-losen Datensatz kein String vorhanden).

## Schritt 2: Button-/Spaltentitel-Image-Name (Stand 2026-08-09)

Datensatz: `WIN_TEST3.TABLE_TEST3_<RRGGBB>_IMAGE_GESETZT.bin` (7 Stück,
Image = `…\primaPOS\Evolution2_Table_ColTitle.png`).

Befund:
- **Kein Klartext-Name.** Weder ASCII noch UTF-16, weder roh noch entpackt;
  auch keine Fragmente (`Evolution`, `ColTitle`, `.png`, `primaPOS`, `bild`).
- **Offset 973 ist NUR ein "Bild vorhanden"-Marker**, NICHT der Name:
  farbunabhängig, fixe Breite, mit `0x57`('W') gepolstert:
  - ohne Bild: `… 57 57 57 3f f9 c2 75 9e 81 38 …`
  - mit Bild:  `… 57 57 57 57 57 57 57 3f 3a d1 3c …`  (4× je Spalte)
  Dieses Token `3f 3a d1 3c` ist bei ALLEN Bildern gleich (Evolution wie
  bild1/2/3) → identifiziert das Bild NICHT.
- **Der echte Pfad/Name ist ein längenvariabler Zeichen-Scramble** ~Offset
  4222. Verifiziert mit `WIN_TEST3.TABLE_TEST3_BILD1..3.bin`
  (Pfad `I:\My Projects\29\primaB2B\bild1.png` .. `bild3.png`):
  genau EIN Byte ändert sich bei 1-Zeichen-Änderung des Dateinamens —
  `'1'`(0x31)→`0x99`, `'2'`(0x32)→`0x8c`, `'3'`(0x33)→`0x13`,
  Kontext stabil `a1 60 __ a5 5c 0c 2f ca`.
- Transform ist **positionsabhängige nichtlineare Substitution**:
  beweisbar KEIN XOR (K wäre inkonsistent), KEIN Affin (Prädiktion für '3'
  schlägt fehl), KEIN Standard-Hash (Hash avalanched, hier 1 Byte).
- Längerer Pfad (Evolution) → andere/größere Region → Feldlänge folgt der
  Pfadlänge (String-Scramble, kein fixer ID-Hash).

Konsequenz Schritt 2 — zwei Wege:
- **(A) Pragmatischer Lookup (empfohlen fürs Ziel):** jedes Bild an seinen
  verwürfelten Feld-Bytes fingerprinten → realer Name. Robust, sofort machbar,
  wie bei den Caption-Positionen. Kollision zweier Namen unwahrscheinlich, da
  das ganze (längenvariable) Feld als Fingerprint dient.
- **(B) Voll-Decode zu Klartext:** Substitution reversen. Braucht gezielten
  Sweep: EINE Zeichenposition über viele Werte variieren (z.B. Dateinamen
  `bilda.png`, `bildb.png`, … oder ein Zeichen-Slot 0..255), um die S-Box je
  Position zu mappen und die Positionsabhängigkeit zu bestimmen.
- Nebenbefund: mit gesetztem Bild wird die Titel-Farbe NICHT mehr in diesen
  Blob serialisiert → `000000_IMAGE` und `FFFFFF_IMAGE` sind byte-identisch
  (Farbe greift visuell über Vererbung).

## Weg B — Scramble reversen: Zwischenstand (Samples bild0/1/2/3/9/A/z, Aild*)

Ziel: den Bildpfad als Klartext lesen. Ergebnis der Analyse:

- **Keine Formel.** 65536 Affin- plus alle XOR/Rotate/Bit-Trick-Varianten
  getestet — keine trifft die 3 Ausgangspunkte. XOR zweier Ausgaben ist
  nichtlinear → echte S-Box, keine Arithmetik.
- **Keine byte-weise Substitution.** Dasselbe Klartext-Zeichen an derselben
  Position ändert je nach Wert VERSCHIEDENE Byte-Positionen:
  `'1'/'2'/'3'` → Byte 4222 (`0x99/0x8c/0x13`), `'9'` → Byte 4221 (`0x5e`,
  4222 bleibt), `'0'` → 4222+4223, `'A'/'z'` → viele Bytes ab 4221.
- **Variabel-lange (bit-gepackte) Kodierung — bestätigt per Realignment:**
  `bild2/3/9` sind nach der Änderung byte-identisch (60/60 Match), aber
  `bild0/A/z` VERSCHIEBEN den nachfolgenden Stream (schlechtes Realignment,
  Länge ändert sich). Verschiedene Zeichen belegen also unterschiedlich viele
  Bits → Huffman-/arithmetik-artiger Bit-Code, NICHT eine Byte-Tabelle.
- **Kein dekodierbarer verschachtelter LZHUF** (Standard-Tabellen, alle
  Startoffsets 4120..4224, fill 0x00/0x20 → kein `bild`/`png`/`Projects`).

Fazit Weg B: Der Pfad ist ein **bit-level variabel-langer Code** (bestätigt
den ursprünglichen "2. Bitstream"-Verdacht). Black-Box-Tabellierung reicht
NICHT, weil der Code bit- und längenabhängig ist und sich pro Position
verschiebt. Realistische Wege zum Klartext:
  1. **Disassembly der WinDev-Serialisierung** (`wd290vm.dll`/`wd290obj.dll`,
     x64dbg/SoftICE) — die EINE Scramble-Routine deckt Farbe, Position UND
     Bildpfad ab (alle aus derselben Obfuskations-Familie). Effizientester Weg.
  2. Voller Bit-Code-Reverse aus sehr vielen Samples — unverhältnismäßig.
Bis dahin bleibt **Weg A** (Fingerprint-Lookup, bereits in der DLL) die
praktische Lösung.

Saubere Datenpunkte (Byte 4221/4222, "bildX.png", gleiche Farbe):
  '0'→(60,22,+a7)  '1'→(60,99)  '2'→(60,8c)  '3'→(60,13)  '9'→(5e,99)
  'A'→(0e,ca,…viele)  'z'→(5e,8c,…viele)

## Werkzeuge

- `wd_table_color_analyse.py` — voller Decode, Region-Dump, Diff, LCP.
- `wd_style/` (Rust) — `WDStyleUncompress` = derselbe volle Decode.
