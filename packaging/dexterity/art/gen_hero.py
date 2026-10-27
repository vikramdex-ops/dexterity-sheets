# Generates docs/readme/hero.svg (animated README banner). Run: py -3 packaging/dexterity/art/gen_hero.py
import os

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "docs", "readme")
FONT = "'Segoe UI', 'Segoe UI Variable Text', Arial, Helvetica, sans-serif"
BLUE, RED, GREEN = "#1f77b4", "#d62728", "#2ca02c"
T = 13  # seconds per loop

a, b, c, d = 0.803, -0.322, 0.563, 0.461
O = (566, 318)


def P(x, y):
    return (O[0] + a * x + c * y, O[1] + b * x + d * y)


def V(dx, dy):
    return (a * dx + c * dy, b * dx + d * dy)


def f(v):
    s = f"{v:.1f}"
    return s[:-2] if s.endswith(".0") else s


def pts(seq):
    return " ".join(f"{f(x)},{f(y)}" for x, y in seq)


COLS = {"#": (0, 34), "A": (34, 170), "B": (170, 236), "C": (236, 312), "D": (312, 428), "E": (428, 560)}
TOP, ROWH, W, H = 26, 36, 560, 350


def cell(col, r):
    x1, x2 = COLS[col]
    y1 = TOP + (r - 1) * ROWH
    return x1, y1, x2, y1 + ROWH


IFC, HOLD = "#17803a", "#b05f00"
DATA = [
    ("100-P-2001", '6"', "A1A", "28.4", "IFC"),
    ("100-P-2002", '4"', "A1A", None, "IFC"),
    ("100-P-2003", '8"', "B2C", "62.0", None),
    ("100-CW-2004", '3"', "C1", "21.7", "IFC"),
    ("100-CW-2005", '3"', "C1", "23.1", "Hold"),
    ("100-ST-2006", None, "D3", "17.4", "IFC"),
    ("100-P-2007", '10"', "A1A", "84.9", "Hold"),
]

css = []
plane = []   # inside matrix group (local coords)
screen = []  # screen space, floats with plane


def txt(col, r, s, anchor="start", cls="", fill="#24324d", weight=None, size=12.5):
    x1, y1, x2, y2 = cell(col, r)
    x = x1 + 8 if anchor == "start" else x2 - 8
    w = f' font-weight="{weight}"' if weight else ""
    cl = f' class="{cls}"' if cls else ""
    a_ = ' text-anchor="end"' if anchor == "end" else ""
    return f'<text x="{f(x)}" y="{f(y1 + 23)}"{a_}{cl}{w} font-size="{size}" fill="{fill}">{s}</text>'


# --- the sheet --------------------------------------------------------------
plane.append(f'<rect x="0" y="-32" width="{W}" height="{H + 32}" rx="7" fill="#fbfcff"/>')
plane.append(f'<path d="M7-32H{W - 7}a7 7 0 0 1 7 7V0H0V-25a7 7 0 0 1 7-7z" fill="#0d3b8c"/>')
plane.append('<text x="14" y="-11" font-size="13" font-weight="700" fill="#fff">Line List \u00b7 Unit 100</text>')
for i, col in enumerate((BLUE, RED, GREEN)):
    plane.append(f'<circle cx="{W - 70 + i * 13}" cy="-16" r="5" fill="{col}" stroke="#0d3b8c" stroke-width="1.5"/>')
plane.append(f'<text x="{W - 12}" y="-11" text-anchor="end" font-size="11" fill="#bcd3ff">3</text>')
plane.append(f'<rect x="0" y="0" width="{W}" height="{TOP}" fill="#e7eef9"/>')
plane.append(f'<rect x="0" y="0" width="34" height="{H}" fill="#e7eef9"/>')
grid = []
for col, (x1, x2) in COLS.items():
    grid.append(f"M{x2} 0V{H}")
for r in range(0, 10):
    y = TOP + r * ROWH
    grid.append(f"M0 {y}H{W}")
plane.append(f'<path d="{"".join(grid)}" stroke="#d3ddee" stroke-width="1" fill="none"/>')
for col, (x1, x2) in COLS.items():
    if col != "#":
        plane.append(f'<text x="{f((x1 + x2) / 2)}" y="18" text-anchor="middle" font-size="11.5" fill="#5d6f91">{col}</text>')
for r in range(1, 10):
    plane.append(f'<text x="17" y="{TOP + (r - 1) * ROWH + 23}" text-anchor="middle" font-size="11" fill="#5d6f91">{r}</text>')
for col, h in zip("ABCDE", ("Line No", "Size", "Spec", "Length (m)", "Status")):
    plane.append(txt(col, 1, h, fill="#0d3b8c", weight="700"))
for i, row in enumerate(DATA):
    r = i + 2
    ln, size, spec, length, status = row
    plane.append(txt("A", r, ln))
    if size:
        plane.append(txt("B", r, size, "end"))
    plane.append(txt("C", r, spec))
    if length:
        plane.append(txt("D", r, length, "end"))
    if status:
        plane.append(txt("E", r, status, fill=IFC if status == "IFC" else HOLD, weight="600"))
plane.append(txt("A", 9, "Total", fill="#0d3b8c", weight="700"))

# --- the three edits ----------------------------------------------------------
# (who, colour, start cell, target cell, old, new, align, newfill, move start/end, type start/end, steps, commit)
EDITS = [
    ("an", "Anita Rao", RED, ("D", 6), ("D", 3), "36.2", "38.0", "end", "#24324d", (5, 13), (15, 23), 4, 25),
    ("ra", "Rahul Mehta", GREEN, ("E", 8), ("E", 4), "Hold", "IFC", "start", IFC, (29, 37), (39, 45), 3, 47),
    ("vi", "Vikram", BLUE, ("B", 3), ("B", 7), "", '2"', "end", "#24324d", (51, 59), (61, 65), 2, 67),
]

history = []
for key, name, colr, start, target, old, new, align, nfill, mv, ty, steps, commit in EDITS:
    tx1, ty1, tx2, ty2 = cell(*target)
    sx1, sy1, sx2, sy2 = cell(*start)
    # new value, its typing cover, then the old value on top
    plane.append(txt(target[0], target[1], new, align, fill=nfill, weight="600" if nfill == IFC else None))
    tw = len(new) * 7.6 + 4
    cx = tx1 + 6 if align == "start" else tx2 - 6 - tw
    plane.append(f'<rect class="cv {key}c" x="{f(cx)}" y="{ty1 + 5}" width="{f(tw)}" height="{ROWH - 10}" fill="#fbfcff"/>')
    if old:
        oldfill = HOLD if old == "Hold" else "#24324d"
        plane.append(f'<rect class="{key}o" x="{tx1 + 1}" y="{ty1 + 1}" width="{tx2 - tx1 - 2}" height="{ROWH - 2}" fill="#fbfcff"/>')
        plane.append(txt(target[0], target[1], old, align, cls=f"{key}o", fill=oldfill, weight="600" if old == "Hold" else None))
    # commit flash
    plane.append(f'<rect class="fl {key}f" x="{tx1}" y="{ty1}" width="{tx2 - tx1}" height="{ROWH}" fill="{colr}"/>')
    # cursor (moves in plane space)
    plane.append(f'<rect class="{key}m" x="{sx1 + 1}" y="{sy1 + 1}" width="{sx2 - sx1 - 2}" height="{ROWH - 2}" rx="2" fill="{colr}" fill-opacity=".13" stroke="{colr}" stroke-width="2.6"/>')
    dx, dy = tx1 - sx1, ty1 - sy1
    m0, m1 = mv
    css.append(f".{key}m{{animation:{key}m {T}s ease-in-out infinite}}")
    css.append(f"@keyframes {key}m{{0%,{m0}%{{transform:translate(0,0)}}{m1}%,88%{{transform:translate({dx}px,{dy}px)}}97%,100%{{transform:translate(0,0)}}}}")
    sdx, sdy = V(dx, dy)
    css.append(f".{key}t{{animation:{key}t {T}s ease-in-out infinite}}")
    css.append(f"@keyframes {key}t{{0%,{m0}%{{transform:translate(0,0)}}{m1}%,88%{{transform:translate({f(sdx)}px,{f(sdy)}px)}}97%,100%{{transform:translate(0,0)}}}}")
    t0, t1 = ty
    css.append(f".{key}c{{animation:{key}c {T}s infinite}}")
    css.append(f"@keyframes {key}c{{0%,{t0}%{{transform:scaleX(1);animation-timing-function:steps({steps},end)}}{t1}%,90%{{transform:scaleX(0)}}91%,100%{{transform:scaleX(1)}}}}")
    css.append(f".{key}o{{animation:{key}o {T}s infinite}}")
    css.append(f"@keyframes {key}o{{0%,{t0 - 1}%{{opacity:1}}{t0}%,90%{{opacity:0}}93%,100%{{opacity:1}}}}")
    css.append(f".{key}f{{animation:{key}f {T}s ease-out infinite}}")
    css.append(f"@keyframes {key}f{{0%,{commit}%{{opacity:0}}{commit + 1}%{{opacity:.5}}{commit + 9}%,100%{{opacity:0}}}}")
    # name tag (screen space) anchored at the start cell's top-left corner
    px, py = P(sx1, sy1)
    tagw = len(name) * 7.3 + 18
    screen.append(
        f'<g transform="translate({f(px - 1.3)},{f(py)})"><g class="{key}t">'
        f'<path d="M1.3 0V-14" stroke="{colr}" stroke-width="2.6"/><g transform="translate(0 -10)">'
        f'<path d="M0 0V-26a5 5 0 0 1 5-5H{f(tagw - 5)}a5 5 0 0 1 5 5V-14a5 5 0 0 1-5 5H9z" fill="{colr}"/>'
        f'<text x="9" y="-14.5" font-size="12.5" font-weight="600" fill="#fff">{name}</text>'
        f'<g class="{key}y"><rect x="{f(tagw + 5)}" y="-30" width="58" height="20" rx="10" fill="#fff"/>'
        f'<text x="{f(tagw + 34)}" y="-15.5" text-anchor="middle" font-size="11.5" font-style="italic" fill="{colr}">typing\u2026</text></g>'
        f"</g></g></g>"
    )
    css.append(f".{key}y{{opacity:0;animation:{key}y {T}s infinite}}")
    css.append(f"@keyframes {key}y{{0%,{t0 - 1}%{{opacity:0}}{t0}%,{t1 + 1}%{{opacity:1}}{t1 + 3}%,100%{{opacity:0}}}}")
    # the committed cell lifts out of the sheet (screen space)
    corners = [P(tx1, ty1), P(tx2, ty1), P(tx2, ty2), P(tx1, ty2)]
    screen.append(f'<polygon class="lf {key}l" points="{pts(corners)}" fill="{colr}" fill-opacity=".35" stroke="{colr}" stroke-width="1.5"/>')
    css.append(f".{key}l{{animation:{key}l {T}s ease-out infinite}}")
    css.append(f"@keyframes {key}l{{0%,{commit}%{{opacity:0;transform:translateY(0)}}{commit + 1}%{{opacity:1}}{commit + 11}%,100%{{opacity:0;transform:translateY(-26px)}}}}")
    history.append((key, name, colr, f"{target[0]}{target[1]}", old or "(empty)", new, commit))

# Total recalculates after Anita's edit
plane.append(txt("D", 9, "275.5", "end", fill="#0d3b8c", weight="700"))
x1, y1, x2, y2 = cell("D", 9)
plane.append(f'<rect class="to" x="{x1 + 1}" y="{y1 + 1}" width="{x2 - x1 - 2}" height="{ROWH - 2}" fill="#fbfcff"/>')
plane.append(txt("D", 9, "273.7", "end", cls="to", fill="#0d3b8c", weight="700"))
css.append(f".to{{animation:to {T}s infinite}}@keyframes to{{0%,26%{{opacity:1}}27%,90%{{opacity:0}}93%,100%{{opacity:1}}}}")
plane.append(f'<rect class="fl tof" x="{x1}" y="{y1}" width="{x2 - x1}" height="{ROWH}" fill="#4d8dff"/>')
css.append(f".tof{{animation:tof {T}s ease-out infinite}}@keyframes tof{{0%,26%{{opacity:0}}27%{{opacity:.35}}34%,100%{{opacity:0}}}}")

# --- stage geometry -----------------------------------------------------------
TL, TR, BL, BR = P(0, -32), P(W, -32), P(0, H), P(W, H)
slab = 10


def off(p, dy):
    return (p[0], p[1] + dy)


shadow = pts([off(TL, 46), off(TR, 46), off(BR, 46), off(BL, 46)])
ghost1 = pts([off(TL, 22), off(TR, 22), off(BR, 22), off(BL, 22)])
ghost2 = pts([off(TL, 42), off(TR, 42), off(BR, 42), off(BL, 42)])
slabp = pts([TL, BL, BR, off(BR, slab), off(BL, slab), off(TL, slab)])
matrix = f"matrix({a} {b} {c} {d} {O[0]} {O[1]})"

# floor grid (perspective), sits below the sheet
floor = []
for gx in range(-700, 1401, 70):
    floor.append(f"M{gx} -700V1000")
for gy in range(-700, 1001, 70):
    floor.append(f"M-700 {gy}H1400")

# history card
hx, hy = 1012, 30
hrows = []
for i, (key, name, colr, ref, old, new, commit) in enumerate(history):
    y = 58 + i * 24
    hrows.append(f'<rect class="sk {key}k" x="14" y="{y - 9}" width="{150 - i * 24}" height="8" rx="4" fill="#dfe7f5"/>')
    hrows.append(
        f'<g class="{key}h"><circle cx="18" cy="{y - 4}" r="4" fill="{colr}"/>'
        f'<text x="28" y="{y}" font-size="12"><tspan font-weight="700" fill="{colr}">{name.split()[0]}</tspan>'
        f'<tspan fill="#6a7896" dx="6">{ref}</tspan><tspan fill="#24324d" dx="6">{old} \u2192 {new}</tspan></text></g>'
    )
    css.append(f".{key}h{{animation:{key}h {T}s ease-out infinite}}")
    css.append(f"@keyframes {key}h{{0%,{commit}%{{opacity:0;transform:translateX(10px)}}{commit + 2}%,90%{{opacity:1;transform:translateX(0)}}94%,100%{{opacity:0}}}}")
    css.append(f".{key}k{{opacity:0;animation:{key}k {T}s infinite}}")
    css.append(f"@keyframes {key}k{{0%,{commit}%{{opacity:1}}{commit + 1}%,91%{{opacity:0}}94%,100%{{opacity:1}}}}")

# particles
import random

random.seed(7)
parts = []
for i in range(16):
    x = random.randint(40, 1240)
    y = random.randint(90, 520)
    s = random.choice((6, 8, 10, 12, 14))
    colr = random.choice(("#ffffff", "#ffffff", "#ffffff", BLUE, RED, GREEN, "#8fb6ff"))
    op = 0.10 if colr == "#ffffff" else 0.5
    dur = random.randint(9, 16)
    delay = -random.randint(0, dur)
    parts.append(
        f'<rect class="pt" style="animation-duration:{dur}s;animation-delay:{delay}s" x="{x}" y="{y}" width="{s}" height="{s}" rx="2" fill="{colr}" fill-opacity="{op}"/>'
    )

ICON = (
    '<g transform="translate(72 74) scale(.125)">'
    '<rect width="512" height="512" rx="112" fill="url(#tile)"/>'
    '<path d="M64 64V448M160 64V448M256 64V448M352 64V448M448 64V448M64 64H448M64 160H448M64 256H448M64 352H448M64 448H448" '
    'stroke="#fff" stroke-opacity=".36" stroke-width="8"/>'
    '<path d="M136 106H260A144 150 0 0 1 260 406H136Z" fill="url(#tile)"/>'
    '<path d="M150 120H260A130 136 0 0 1 260 392H150ZM222 184H260A60 72 0 0 1 260 328H222Z" fill="#fff" fill-rule="evenodd"/>'
    '<rect width="512" height="512" rx="112" fill="none" stroke="#fff" stroke-opacity=".25" stroke-width="10"/></g>'
)

chips = []
cx0 = 72
for label, colr in (("Live editing", "#4fa3ff"), ("Every change signed", "#ff6b6b"), ("Opens .xlsx", "#56d364")):
    w = len(label) * 7.6 + 40
    chips.append(
        f'<g transform="translate({f(cx0)} 352)"><rect width="{f(w)}" height="32" rx="16" fill="#ffffff" fill-opacity=".08" stroke="#ffffff" stroke-opacity=".2"/>'
        f'<circle cx="17" cy="16" r="4.5" fill="{colr}"/><text x="30" y="21" font-size="14.5" font-weight="600" fill="#e3ecff">{label}</text></g>'
    )
    cx0 += w + 10

base_css = f"""
text{{font-family:{FONT}}}
.fl,.lf,.to{{opacity:0}}
.cv{{transform-box:fill-box;transform-origin:100% 50%;transform:scaleX(0)}}
.ano,.rao,.vio{{opacity:0}}
.flt{{animation:flt 11s ease-in-out infinite;transform-origin:880px 300px}}
@keyframes flt{{0%,100%{{transform:translateY(0) rotate(0)}}50%{{transform:translateY(-9px) rotate(-.7deg)}}}}
.crd{{animation:crd 8s ease-in-out infinite}}
@keyframes crd{{0%,100%{{transform:translateY(0)}}50%{{transform:translateY(-12px)}}}}
.pre{{animation:crd 9s ease-in-out -3s infinite}}
.shd{{animation:shd 11s ease-in-out infinite;transform-origin:880px 420px}}
@keyframes shd{{0%,100%{{opacity:.55;transform:scale(1)}}50%{{opacity:.4;transform:scale(.97)}}}}
.in{{animation:in 1s cubic-bezier(.2,.7,.2,1) both}}
@keyframes in{{from{{opacity:0;transform:translateY(16px)}}}}
.i2{{animation-delay:.15s}}.i3{{animation-delay:.3s}}.i4{{animation-delay:.45s}}.i5{{animation-delay:.6s}}.i6{{animation-delay:.75s}}
.stg{{animation:stg 1.6s cubic-bezier(.2,.7,.2,1) both}}
@keyframes stg{{from{{opacity:0;transform:translate(60px,70px)}}}}
.sw{{animation:sw 9s ease-in-out 1s infinite;transform:translateX(-420px) skewX(-20deg)}}
@keyframes sw{{0%{{transform:translateX(-420px) skewX(-20deg)}}55%,100%{{transform:translateX(1500px) skewX(-20deg)}}}}
.pt{{animation:pt 12s linear infinite}}
@keyframes pt{{0%{{transform:translateY(24px);opacity:0}}20%,75%{{opacity:1}}100%{{transform:translateY(-70px);opacity:0}}}}
.btn{{animation:in 1s cubic-bezier(.2,.7,.2,1) .6s both}}
.glw{{opacity:0;animation:glw 3.2s ease-in-out infinite}}
@keyframes glw{{0%,100%{{opacity:.0}}50%{{opacity:.55}}}}
"""
reduce_css = "@media (prefers-reduced-motion:reduce){*{animation:none!important}.sw{display:none}}"

svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1280 560" width="1280" height="560" role="img" aria-labelledby="t d">
<title id="t">Dexterity Sheets</title>
<desc id="d">Your team's spreadsheet. Live, together, on the office network. A line list floats in 3D while Vikram, Anita Rao and Rahul Mehta edit it together; each change is written into the Edit History with their name.</desc>
<!-- Original artwork by Dexterity Design Services Pvt. Ltd. -->
<defs>
<style>{base_css}{''.join(css)}{reduce_css}</style>
<linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#0d3b8c"/><stop offset=".55" stop-color="#082866"/><stop offset="1" stop-color="#04163d"/></linearGradient>
<radialGradient id="glow" cx="900" cy="270" r="560" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#3b7bff" stop-opacity=".45"/><stop offset="1" stop-color="#3b7bff" stop-opacity="0"/></radialGradient>
<radialGradient id="glow2" cx="180" cy="560" r="420" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#1f77b4" stop-opacity=".35"/><stop offset="1" stop-color="#1f77b4" stop-opacity="0"/></radialGradient>
<radialGradient id="ff" cx="880" cy="330" r="430" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#fff"/><stop offset="1" stop-color="#000"/></radialGradient>
<mask id="fm" maskUnits="userSpaceOnUse" x="0" y="0" width="1280" height="560"><rect width="1280" height="560" fill="url(#ff)"/></mask>
<linearGradient id="swg" x1="0" x2="1"><stop offset="0" stop-color="#fff" stop-opacity="0"/><stop offset=".5" stop-color="#fff" stop-opacity=".13"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></linearGradient>
<linearGradient id="tile" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2="0" y2="512"><stop offset="0" stop-color="#2a63c9"/><stop offset="1" stop-color="#0d3b8c"/></linearGradient>
<linearGradient id="btn" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#dfe9ff"/></linearGradient>
<linearGradient id="sl" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#9db8e6"/><stop offset="1" stop-color="#5b7fc0"/></linearGradient>
<filter id="blur" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="20"/></filter>
<filter id="ds" x="-20%" y="-20%" width="140%" height="160%"><feDropShadow dx="0" dy="10" stdDeviation="12" flood-color="#020a1f" flood-opacity=".55"/></filter>
<clipPath id="stage"><rect width="1280" height="560" rx="28"/></clipPath>
</defs>
<g clip-path="url(#stage)">
<rect width="1280" height="560" fill="url(#bg)"/>
<rect width="1280" height="560" fill="url(#glow)"/>
<rect width="1280" height="560" fill="url(#glow2)"/>
<g mask="url(#fm)" opacity=".5"><path transform="translate(0 70) {matrix}" d="{''.join(floor)}" stroke="#8fb6ff" stroke-opacity=".22" stroke-width="1.4" fill="none"/></g>
{''.join(parts)}
<g class="stg">
<polygon class="shd" points="{shadow}" fill="#000" filter="url(#blur)"/>
<g class="flt">
<polygon points="{ghost2}" fill="#8fb6ff" fill-opacity=".05" stroke="#bcd3ff" stroke-opacity=".18"/>
<polygon points="{ghost1}" fill="#8fb6ff" fill-opacity=".08" stroke="#bcd3ff" stroke-opacity=".3"/>
<polygon points="{slabp}" fill="url(#sl)"/>
<g transform="{matrix}">{''.join(plane)}</g>
{''.join(screen)}
</g>
</g>
<g class="crd"><g transform="translate({hx} {hy})" filter="url(#ds)">
<rect width="236" height="136" rx="12" fill="#fff"/>
<text x="14" y="25" font-size="13.5" font-weight="700" fill="#0d3b8c">Edit History</text>
<circle cx="181" cy="20.5" r="4" fill="#2ca02c"/><circle class="glw" cx="181" cy="20.5" r="8" fill="#2ca02c"/>
<text x="222" y="25" text-anchor="end" font-size="11.5" fill="#56657f">Live</text>
<path d="M14 36H222" stroke="#e3e9f4"/>
{''.join(hrows)}
<text x="14" y="127" font-size="10.5" fill="#7a88a3">who \u00b7 cell \u00b7 old \u2192 new</text>
</g></g>
<g class="pre"><g transform="translate(840 508)">
<rect width="392" height="36" rx="18" fill="#04122f" fill-opacity=".55" stroke="#fff" stroke-opacity=".16"/>
<circle cx="20" cy="18" r="4.5" fill="#56d364"/><text x="31" y="23" font-size="13" fill="#dbe6ff">Shared \u00b7 saved</text>
<circle cx="150" cy="18" r="4.5" fill="{BLUE}"/><text x="161" y="23" font-size="13" fill="#dbe6ff">Vikram</text>
<circle cx="225" cy="18" r="4.5" fill="#ff5c5c"/><text x="236" y="23" font-size="13" fill="#dbe6ff">Anita Rao</text>
<circle cx="310" cy="18" r="4.5" fill="#4fd44f"/><text x="321" y="23" font-size="13" fill="#dbe6ff">Rahul M.</text>
</g></g>
<g class="in">{ICON}<text x="152" y="101" font-size="13" font-weight="600" letter-spacing="2.4" fill="#9fbcf0">DEXTERITY DESIGN SERVICES</text>
<text x="152" y="124" font-size="14" fill="#7f9bd0">The office spreadsheet</text></g>
<text class="in i2" x="70" y="226" font-size="60" font-weight="700" fill="#fff" letter-spacing="-1">Dexterity Sheets</text>
<text class="in i3" x="72" y="278" font-size="28" font-weight="600" fill="#ffffff">Your team's spreadsheet.</text>
<text class="in i4" x="72" y="314" font-size="22" fill="#9cc2ff">Live, together, on the office network.</text>
<g class="in i5">{''.join(chips)}</g>
<g class="btn"><g transform="translate(72 410)">
<rect width="252" height="50" rx="25" fill="url(#btn)"/>
<path d="M26 15v14m-6-6 6 6 6-6M18 34h16" stroke="#0d3b8c" stroke-width="2.6" fill="none" stroke-linecap="round" stroke-linejoin="round"/>
<text x="46" y="31" font-size="16.5" font-weight="700" fill="#0d3b8c">Download for Windows</text></g>
<text x="342" y="432" font-size="13.5" fill="#b9cdf2">Windows 10 &amp; 11</text>
<text x="342" y="451" font-size="13.5" fill="#8ea7d6">No admin rights needed</text></g>
<text class="in i6" x="72" y="508" font-size="13.5" fill="#7f98c8">Edit History \u00b7 Versions \u00b7 Opens and saves Excel .xlsx \u00b7 Updates itself</text>
<rect class="sw" x="0" y="-80" width="240" height="720" fill="url(#swg)"/>
</g>
<rect x=".75" y=".75" width="1278.5" height="558.5" rx="27.5" fill="none" stroke="#fff" stroke-opacity=".12" stroke-width="1.5"/>
</svg>
"""

os.makedirs(OUT, exist_ok=True)
with open(os.path.join(OUT, "hero.svg"), "w", encoding="utf-8", newline="\n") as fh:
    fh.write(svg)
print("hero.svg", len(svg.encode("utf-8")))
