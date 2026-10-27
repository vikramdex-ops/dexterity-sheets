# Generates docs/readme/steps.svg and feature-*.svg. Run: py -3 packaging/dexterity/art/gen_cards.py
import os

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "docs", "readme")
FONT = "'Segoe UI', 'Segoe UI Variable Text', Arial, Helvetica, sans-serif"
BLUE, RED, GREEN = "#1f77b4", "#d62728", "#2ca02c"
LBLUE, LRED, LGREEN = "#4fa3ff", "#ff6b6b", "#56d364"
REDUCE = "@media (prefers-reduced-motion:reduce){*{animation:none!important}}"


def f(v):
    s = f"{v:.1f}"
    return s[:-2] if s.endswith(".0") else s


def card(name, title, sub, desc, art, css, extra_defs=""):
    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 260" width="400" height="260" role="img" aria-labelledby="t d">
<title id="t">{title}</title>
<desc id="d">{sub} {desc}</desc>
<!-- Original artwork by Dexterity Design Services Pvt. Ltd. -->
<defs>
<style>text{{font-family:{FONT}}}{css}{REDUCE}</style>
<linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#0f428f"/><stop offset=".6" stop-color="#082866"/><stop offset="1" stop-color="#051a45"/></linearGradient>
<radialGradient id="gl" cx="200" cy="90" r="190" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#3b7bff" stop-opacity=".42"/><stop offset="1" stop-color="#3b7bff" stop-opacity="0"/></radialGradient>
<filter id="ds" x="-30%" y="-30%" width="160%" height="170%"><feDropShadow dx="0" dy="8" stdDeviation="9" flood-color="#020a1f" flood-opacity=".5"/></filter>
<clipPath id="cl"><rect width="400" height="260" rx="20"/></clipPath>
{extra_defs}
</defs>
<g clip-path="url(#cl)">
<rect width="400" height="260" fill="url(#bg)"/>
<rect width="400" height="260" fill="url(#gl)"/>
<path d="M0 184H400" stroke="#fff" stroke-opacity=".08"/>
{art}
<text x="200" y="217" text-anchor="middle" font-size="21" font-weight="700" fill="#fff">{title}</text>
<text x="200" y="242" text-anchor="middle" font-size="14.5" fill="#a9c6f7">{sub}</text>
</g>
<rect x=".75" y=".75" width="398.5" height="258.5" rx="19.5" fill="none" stroke="#fff" stroke-opacity=".12" stroke-width="1.5"/>
</svg>
"""
    path = os.path.join(OUT, name)
    with open(path, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(svg)
    print(name, len(svg.encode("utf-8")))


def iso(k, ox, oy):
    a, b, c, d = 0.803 * k, -0.322 * k, 0.563 * k, 0.461 * k
    return (a, b, c, d), (lambda x, y: (ox + a * x + c * y, oy + b * x + d * y)), (lambda dx, dy: (a * dx + c * dy, b * dx + d * dy)), f"matrix({f2(a)} {f2(b)} {f2(c)} {f2(d)} {ox} {oy})"


def f2(v):
    return f"{v:.3f}".rstrip("0").rstrip(".")


def pts(seq):
    return " ".join(f"{f(x)},{f(y)}" for x, y in seq)


def tag(x, y, name, colr, cls=""):
    w = len(name) * 6.9 + 16
    c = f' class="{cls}"' if cls else ""
    return (
        f'<g transform="translate({f(x)} {f(y)})"><g{c}><path d="M1.2 0V-10" stroke="{colr}" stroke-width="2.4"/>'
        f'<rect x="0" y="-30" width="{f(w)}" height="21" rx="5" fill="{colr}"/>'
        f'<text x="8" y="-15" font-size="12" font-weight="600" fill="#fff">{name}</text></g></g>'
    )


# ---------------------------------------------------------------- feature-live
def live():
    T = 7
    (a, b, c, d), P, V, M = iso(0.9, 92, 124)
    cw, rh, nc, nr = 48, 30, 5, 4
    W, H = cw * nc, rh * nr
    plane = [f'<rect x="0" y="-16" width="{W}" height="{H + 16}" rx="5" fill="#fbfcff"/>',
             f'<path d="M5-16H{W - 5}a5 5 0 0 1 5 5V0H0V-11a5 5 0 0 1 5-5z" fill="#0d3b8c"/>']
    for i, col in enumerate((BLUE, RED, GREEN)):
        plane.append(f'<circle cx="{W - 40 + i * 11}" cy="-8" r="3.6" fill="{col}"/>')
    g = "".join(f"M{x * cw} 0V{H}" for x in range(1, nc)) + "".join(f"M0 {y * rh}H{W}" for y in range(1, nr))
    plane.append(f'<path d="{g}" stroke="#d3ddee"/>')
    import random
    random.seed(3)
    for r in range(nr):
        for col in range(nc):
            if (r, col) in ((1, 3), (2, 1), (3, 4)):
                continue
            w = random.choice((18, 24, 30, 34))
            fill = "#0d3b8c" if r == 0 else "#b9c7de"
            plane.append(f'<rect x="{col * cw + 7}" y="{r * rh + 12}" width="{w}" height="6" rx="3" fill="{fill}"/>')
    css = []
    screen = []
    # (key, name, colour, from cell, to cell, commit %, bar width)
    moves = [("a", "Anita", RED, LRED, (0, 1), (3, 1), 30, 30), ("r", "Rahul", GREEN, LGREEN, (4, 3), (1, 2), 52, 24), ("v", "Vikram", BLUE, LBLUE, (2, 0), (4, 3), 74, 34)]
    for key, nm, colr, light, (sc, sr), (tc, tr), cm, bw in moves:
        dx, dy = (tc - sc) * cw, (tr - sr) * rh
        m0, m1 = cm - 20, cm - 10
        plane.append(f'<rect class="{key}n" x="{tc * cw + 7}" y="{tr * rh + 12}" width="{bw}" height="6" rx="3" fill="{colr}"/>')
        plane.append(f'<rect class="{key}f" x="{tc * cw}" y="{tr * rh}" width="{cw}" height="{rh}" fill="{colr}"/>')
        plane.append(f'<rect class="{key}m" x="{sc * cw + 1}" y="{sr * rh + 1}" width="{cw - 2}" height="{rh - 2}" rx="2" fill="{colr}" fill-opacity=".14" stroke="{colr}" stroke-width="2.4"/>')
        px, py = P(sc * cw, sr * rh)
        sdx, sdy = V(dx, dy)
        screen.append(tag(px - 1.2, py, nm, colr, key + "t"))
        css.append(f".{key}m{{animation:{key}m {T}s ease-in-out infinite}}@keyframes {key}m{{0%,{m0}%{{transform:translate(0,0)}}{m1}%,90%{{transform:translate({dx}px,{dy}px)}}98%,100%{{transform:translate(0,0)}}}}")
        css.append(f".{key}t{{animation:{key}t {T}s ease-in-out infinite}}@keyframes {key}t{{0%,{m0}%{{transform:translate(0,0)}}{m1}%,90%{{transform:translate({f(sdx)}px,{f(sdy)}px)}}98%,100%{{transform:translate(0,0)}}}}")
        css.append(f".{key}n{{animation:{key}n {T}s infinite}}@keyframes {key}n{{0%,{cm - 1}%{{opacity:0}}{cm}%,91%{{opacity:1}}94%,100%{{opacity:0}}}}")
        css.append(f".{key}f{{opacity:0;animation:{key}f {T}s ease-out infinite}}@keyframes {key}f{{0%,{cm - 1}%{{opacity:0}}{cm}%{{opacity:.5}}{cm + 8}%,100%{{opacity:0}}}}")
    TL, TR, BL, BR = P(0, -16), P(W, -16), P(0, H), P(W, H)
    slab = pts([TL, BL, BR, (BR[0], BR[1] + 7), (BL[0], BL[1] + 7), (TL[0], TL[1] + 7)])
    shadow = pts([(x, y + 22) for x, y in (TL, TR, BR, BL)])
    css.append(".fl{animation:fl 6s ease-in-out infinite}@keyframes fl{0%,100%{transform:translateY(0)}50%{transform:translateY(-6px)}}")
    art = (
        f'<polygon points="{shadow}" fill="#000" opacity=".45" filter="url(#bl)"/>'
        f'<g class="fl"><polygon points="{slab}" fill="#6f8fca"/><g transform="{M}">{"".join(plane)}</g>{"".join(screen)}</g>'
    )
    card("feature-live.svg", "Share any file, live", "Your file stays put and keeps everyone's edits.",
         "Anita, Rahul and Vikram move around one shared sheet; each value they type appears for everyone.",
         art, "".join(css), '<filter id="bl" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="9"/></filter>')


# ----------------------------------------------------------------- feature-who
def who():
    T = 9
    css = [f".ph{{animation:ph {T}s infinite}}.q2{{animation-delay:-6s}}.q3{{animation-delay:-3s}}.q2,.q3{{opacity:0}}",
           "@keyframes ph{0%{opacity:0}3%,30%{opacity:1}34%,100%{opacity:0}}",
           f".rg{{animation:rg {T}s infinite;transform-box:fill-box;transform-origin:center}}.rg.q2,.rg.q3{{opacity:0}}",
           "@keyframes rg{0%{opacity:0;transform:scale(.8)}4%{opacity:1;transform:scale(1)}30%{opacity:1;transform:scale(1)}34%,100%{opacity:0;transform:scale(1.25)}}",
           ".q2{animation-delay:-6s}.q3{animation-delay:-3s}",
           ".sig{animation:sig 3s ease-in-out infinite}@keyframes sig{0%,100%{transform:translateY(0)}50%{transform:translateY(-4px)}}"]
    people = [("A", "Anita Rao", RED, LRED, "38.0", "07:15"), ("R", "Rahul Mehta", GREEN, LGREEN, "IFC", "07:16"), ("V", "Vikram", BLUE, LBLUE, '2"', "07:18")]
    art = []
    # avatars (signed-in people)
    for i, (ini, nm, colr, light, val, tm) in enumerate(people):
        y = 46 + i * 46
        q = f"q{i + 1}"
        art.append(f'<circle class="rg {q}" cx="62" cy="{y}" r="22" fill="none" stroke="{light}" stroke-width="2.5"/>')
        art.append(f'<circle cx="62" cy="{y}" r="17" fill="{colr}"/><text x="62" y="{y + 6}" text-anchor="middle" font-size="16" font-weight="700" fill="#fff">{ini}</text>')
        art.append(f'<circle cx="74" cy="{y + 12}" r="6.5" fill="#fff"/><path d="M71 {y + 12}l2 2.2 4-4.4" stroke="{colr}" stroke-width="1.8" fill="none" stroke-linecap="round" stroke-linejoin="round"/>')
    # the cell
    cell = ['<g filter="url(#ds)"><rect x="128" y="52" width="216" height="62" rx="8" fill="#fbfcff"/></g>',
            '<path d="M128 72H344M200 52V114M272 52V114" stroke="#d3ddee"/>',
            '<path d="M136 52H336a8 8 0 0 1 8 8V72H128V60a8 8 0 0 1 8-8z" fill="#e7eef9"/>',
            '<text x="138" y="99" font-size="14" fill="#24324d">A1A</text><text x="334" y="99" text-anchor="end" font-size="14" font-weight="600" fill="#17803a">IFC</text>']
    art += cell
    for i, (ini, nm, colr, light, val, tm) in enumerate(people):
        q = f"q{i + 1}"
        art.append(
            f'<g class="ph {q}"><rect x="201.5" y="73.5" width="69" height="39" rx="2" fill="#fbfcff" stroke="{colr}" stroke-width="3"/>'
            f'<rect x="201.5" y="73.5" width="69" height="39" rx="2" fill="{colr}" fill-opacity=".12"/>'
            f'<text x="262" y="99" text-anchor="end" font-size="15" font-weight="700" fill="#24324d">{val}</text>'
            f'{tag(201.5, 73.5, nm, colr)}'
            f'<rect x="128" y="130" width="216" height="28" rx="14" fill="#04122f" fill-opacity=".6" stroke="#fff" stroke-opacity=".16"/>'
            f'<circle cx="144" cy="144" r="4" fill="{light}"/>'
            f'<text x="155" y="148.5" font-size="12" fill="#dbe6ff">Last changed by <tspan font-weight="700" fill="{light}">{nm.split()[0]}</tspan>, {tm}</text></g>'
        )
    card("feature-who.svg", "Every change has a name", "Everyone signs in, so you always know who.",
         "A cell shows who last changed it: Anita Rao, then Rahul Mehta, then Vikram, each with the time.",
         "".join(art), "".join(css))


# ------------------------------------------------------------- feature-history
def history():
    T = 8
    rows = [("Anita Rao", RED, "07:15", "D3", "36.2", "38.0"), ("Rahul Mehta", GREEN, "07:16", "E4", "Hold", "IFC"), ("Vikram", BLUE, "07:18", "B7", "(empty)", '2"')]
    css = [".pn{animation:pn 6s ease-in-out infinite}@keyframes pn{0%,100%{transform:translateY(0)}50%{transform:translateY(-6px)}}"]
    art = ['<g class="pn"><g filter="url(#ds)"><rect x="70" y="18" width="260" height="154" rx="12" fill="#fff"/></g>',
           '<text x="86" y="41" font-size="14" font-weight="700" fill="#0d3b8c">Edit History</text>',
           '<rect x="236" y="27" width="80" height="20" rx="10" fill="#eef3fb" stroke="#d3ddee"/><text x="247" y="41" font-size="11" fill="#45536e">Everyone</text><path d="M298 35l4 4 4-4" stroke="#45536e" stroke-width="1.5" fill="none"/>',
           '<path d="M84 54H316" stroke="#e3e9f4"/>']
    for i, (nm, colr, tm, ref, old, new) in enumerate(rows):
        y = 74 + i * 34
        t0 = 8 + i * 22
        k = f"h{i}"
        art.append(f'<rect class="{k}s" x="86" y="{y - 10}" width="{170 - i * 30}" height="8" rx="4" fill="#e6ecf6"/><rect class="{k}s" x="86" y="{y + 4}" width="90" height="7" rx="3.5" fill="#eef2f9"/>')
        art.append(
            f'<g class="{k}"><circle cx="90" cy="{y - 4}" r="4" fill="{colr}"/>'
            f'<text x="100" y="{y}" font-size="12.5" font-weight="700" fill="{colr}">{nm}</text>'
            f'<text x="314" y="{y}" text-anchor="end" font-size="11.5" fill="#7a88a3">{tm}</text>'
            f'<text x="100" y="{y + 15}" font-size="12" fill="#24324d"><tspan fill="#17803a">Sheet1!{ref}</tspan><tspan dx="8">{old}</tspan> \u2192 <tspan font-weight="700">{new}</tspan></text></g>'
        )
        if i < 2:
            art.append(f'<path d="M84 {y + 22}H316" stroke="#eef2f9"/>')
        css.append(f".{k}{{animation:{k} {T}s cubic-bezier(.2,.7,.2,1) infinite}}@keyframes {k}{{0%,{t0}%{{opacity:0;transform:translateX(14px)}}{t0 + 6}%,88%{{opacity:1;transform:translateX(0)}}94%,100%{{opacity:0}}}}")
        css.append(f".{k}s{{opacity:0;animation:{k}s {T}s infinite}}@keyframes {k}s{{0%,{t0}%{{opacity:1}}{t0 + 3}%,90%{{opacity:0}}95%,100%{{opacity:1}}}}")
    art.append("</g>")
    card("feature-history.svg", "Edit History", "Old value, new value, who and when.",
         "An Edit History panel fills with changes by Anita Rao, Rahul Mehta and Vikram.",
         "".join(art), "".join(css))


# ------------------------------------------------------------ feature-versions
def versions():
    T = 9
    (a, b, c, d), P, V, M = iso(0.7, 36, 67)
    W, H = 200, 120
    labels = [("Rev A", "Mon 10:00"), ("Rev B", "Tue 16:30"), ("Issued for review", "Now")]
    css = [f".lv{{animation:lv {T}s infinite}}.v1{{animation-delay:-6s}}.v0{{animation-delay:-3s}}.lv{{opacity:0}}.lv.v2{{opacity:1}}",
           "@keyframes lv{0%{opacity:0}4%,30%{opacity:1}34%,100%{opacity:0}}"]
    art = []
    for i in range(3):
        dz = (2 - i) * 34  # 0 = top layer drawn last
        layer = []
        g = "".join(f"M{x * 40} 0V{H}" for x in range(1, 5)) + "".join(f"M0 {y * 24}H{W}" for y in range(1, 5))
        op = (0.55, 0.78, 1)[i]
        layer.append(f'<rect width="{W}" height="{H}" rx="5" fill="#fbfcff" fill-opacity="{op}"/>')
        layer.append(f'<path d="{g}" stroke="#c9d5ea"/>')
        layer.append(f'<rect width="{W}" height="12" rx="3" fill="#0d3b8c" fill-opacity="{op}"/>')
        for r in range(1, 5):
            for col in range(5):
                if (r + col + i) % 3:
                    layer.append(f'<rect x="{col * 40 + 6}" y="{r * 24 + 9}" width="{22 + (r * 7 + col * 5 + i * 3) % 12}" height="5" rx="2.5" fill="#b9c7de"/>')
        art.append(f'<g transform="translate(0 {dz})"><g transform="{M}">{"".join(layer)}</g></g>')
        # highlight outline on the active version
        corners = [P(0, 0), P(W, 0), P(W, H), P(0, H)]
        art.append(f'<polygon class="lv v{i}" points="{pts([(x, y + dz) for x, y in corners])}" fill="none" stroke="#ffd166" stroke-width="2.5" stroke-linejoin="round"/>')
    # labels on the right (upright)
    for i, (lab, when) in enumerate(labels):
        dz = (2 - i) * 34
        x, y = P(W, H)
        y += dz
        art.append(f'<path d="M{f(x + 4)} {f(y)}H{f(x + 22)}" stroke="#fff" stroke-opacity=".35"/>')
        art.append(f'<text x="{f(x + 28)}" y="{f(y + 4)}" font-size="12.5" font-weight="600" fill="#e3ecff">{lab}<tspan font-weight="400" fill="#8ea7d6" dx="6">{when}</tspan></text>')
        art.append(f'<circle class="lv v{i}" cx="{f(x + 13)}" cy="{f(y)}" r="4" fill="#ffd166"/>')
    art.append('<g transform="translate(210 146)"><rect width="100" height="28" rx="14" fill="#ffd166"/>'
               '<path d="M17.5 9.5A6 6 0 1 1 15.6 16.6" stroke="#3a2a00" stroke-width="2" fill="none" stroke-linecap="round"/><path d="M13.5 6.5l4 3-3.2 3.6" stroke="#3a2a00" stroke-width="2" fill="none" stroke-linecap="round" stroke-linejoin="round"/>'
               '<text x="34" y="18.5" font-size="12" font-weight="700" fill="#3a2a00">Restore</text></g>')
    card("feature-versions.svg", "Versions", "Kept every hour and on Save. Restore any one.",
         "Three saved versions of a workbook are stacked: Rev A, Rev B and Issued for review, with a Restore button.",
         "".join(art), "".join(css))


# --------------------------------------------------------------- feature-excel
def excel():
    css = [".dc{animation:dc 5s ease-in-out infinite}@keyframes dc{0%,100%{transform:translateY(0)}50%{transform:translateY(-6px)}}",
           ".mi,.mo{animation:mv 2.4s linear infinite}.mo{animation-name:mo}",
           "@keyframes mv{0%{transform:translateX(0);opacity:0}15%{opacity:1}80%{opacity:1}100%{transform:translateX(66px);opacity:0}}",
           "@keyframes mo{0%{transform:translateX(0);opacity:0}15%{opacity:1}80%{opacity:1}100%{transform:translateX(66px);opacity:0}}",
           ".k2{animation-delay:-.8s}.k3{animation-delay:-1.6s}"]
    art = []
    # arrows
    for x0, lab in ((52, "Open"), (262, "Save as")):
        art.append(f'<path d="M{x0} 92H{x0 + 76}" stroke="#8fb6ff" stroke-opacity=".5" stroke-width="2" stroke-dasharray="4 5"/>'
                   f'<path d="M{x0 + 70} 85l8 7-8 7" stroke="#8fb6ff" stroke-width="2.4" fill="none" stroke-linecap="round" stroke-linejoin="round"/>'
                   f'<text x="{x0 + 38}" y="128" text-anchor="middle" font-size="12.5" font-weight="600" fill="#a9c6f7">{lab}</text>')
        cls = "mi" if x0 < 200 else "mo"
        for k in ("", " k2", " k3"):
            art.append(f'<rect class="{cls}{k}" x="{x0}" y="86" width="12" height="12" rx="2.5" fill="#fff" fill-opacity=".85"/>')
    # document
    art.append('<g class="dc"><g filter="url(#ds)"><path d="M150 26H226L254 54V156a6 6 0 0 1-6 6H150a6 6 0 0 1-6-6V32a6 6 0 0 1 6-6z" fill="#fbfcff"/></g>'
               '<path d="M226 26V48a6 6 0 0 0 6 6H254" fill="#dbe5f5"/>'
               '<path d="M156 66H242M156 86H242M156 106H242M156 126H242M184 66V140M213 66V140" stroke="#c9d5ea"/>'
               '<rect x="156" y="58" width="86" height="8" rx="2" fill="#0d3b8c" fill-opacity=".85"/>'
               '<rect x="161" y="73" width="18" height="6" rx="3" fill="#b9c7de"/><rect x="189" y="73" width="20" height="6" rx="3" fill="#b9c7de"/><rect x="218" y="93" width="18" height="6" rx="3" fill="#b9c7de"/><rect x="161" y="113" width="16" height="6" rx="3" fill="#b9c7de"/><rect x="189" y="93" width="14" height="6" rx="3" fill="#b9c7de"/>'
               '<rect x="132" y="132" width="64" height="26" rx="7" fill="#0d3b8c" stroke="#fff" stroke-width="2"/>'
               '<text x="164" y="150.5" text-anchor="middle" font-size="15" font-weight="700" fill="#fff">.xlsx</text></g>')
    card("feature-excel.svg", "Excel files in and out", "Open the .xlsx a client sends. Save .xlsx back.",
         "Cells flow into a workbook file marked .xlsx and flow out again.",
         "".join(art), "".join(css))


# ------------------------------------------------------------- feature-updates
def updates():
    T = 6
    r = 40
    circ = 2 * 3.14159 * r
    css = [f".rgp{{stroke-dasharray:{f(circ)};animation:rgp {T}s ease-in-out infinite}}",
           f"@keyframes rgp{{0%,14%{{stroke-dashoffset:{f(circ)}}}62%,90%{{stroke-dashoffset:0}}96%,100%{{stroke-dashoffset:{f(circ)};opacity:0}}}}",
           f".ar{{opacity:0;animation:ar {T}s ease-in-out infinite}}@keyframes ar{{0%,12%{{opacity:1;transform:translateY(-5px)}}20%,36%,52%{{opacity:1;transform:translateY(3px)}}28%,44%{{opacity:1;transform:translateY(-3px)}}62%,100%{{opacity:0;transform:translateY(3px)}}}}",
           f".ck{{animation:ck {T}s ease-out infinite;stroke-dasharray:60}}@keyframes ck{{0%,62%{{stroke-dashoffset:60;opacity:1}}70%,90%{{stroke-dashoffset:0;opacity:1}}96%,100%{{opacity:0}}}}",
           f".b1{{opacity:0;animation:b1 {T}s infinite}}@keyframes b1{{0%,62%{{opacity:1}}64%,95%{{opacity:0}}98%,100%{{opacity:1}}}}",
           f".b2{{animation:b2 {T}s infinite}}@keyframes b2{{0%,62%{{opacity:0}}64%,95%{{opacity:1}}98%,100%{{opacity:0}}}}",
           f".pr{{animation:pr {T}s infinite;transform-box:fill-box;transform-origin:center}}@keyframes pr{{0%,6%,12%,100%{{transform:scale(1)}}9%{{transform:scale(.93)}}}}",
           f".cu{{opacity:0;animation:cu {T}s ease-in-out infinite}}@keyframes cu{{0%{{transform:translate(26px,26px);opacity:0}}4%{{opacity:1}}8%,11%{{transform:translate(0,0);opacity:1}}20%,100%{{transform:translate(18px,30px);opacity:0}}}}",
           ".dt{stroke-dasharray:2 8;animation:dt .9s linear infinite}@keyframes dt{to{stroke-dashoffset:-10}}"]
    art = [
        # office server
        '<g transform="translate(40 50)"><rect width="64" height="84" rx="8" fill="#0b2a63" stroke="#8fb6ff" stroke-opacity=".5" stroke-width="1.5"/>'
        '<rect x="10" y="12" width="44" height="14" rx="3" fill="#163e86"/><rect x="10" y="34" width="44" height="14" rx="3" fill="#163e86"/><rect x="10" y="56" width="44" height="14" rx="3" fill="#163e86"/>'
        '<circle cx="46" cy="19" r="2.5" fill="#56d364"/><circle cx="46" cy="41" r="2.5" fill="#56d364"/><circle cx="46" cy="63" r="2.5" fill="#4fa3ff"/>'
        '<text x="32" y="104" text-anchor="middle" font-size="11.5" fill="#a9c6f7">Office server</text></g>',
        '<path class="dt" d="M112 92H166" stroke="#bcd3ff" stroke-width="3" stroke-linecap="round"/>',
        # ring
        f'<circle cx="214" cy="92" r="{r}" fill="#061a45" stroke="#fff" stroke-opacity=".14" stroke-width="7"/>',
        f'<circle class="rgp" cx="214" cy="92" r="{r}" fill="none" stroke="#4fa3ff" stroke-width="7" stroke-linecap="round" transform="rotate(-90 214 92)"/>',
        '<g class="ar"><path d="M214 74v30m-11-11 11 11 11-11" stroke="#fff" stroke-width="4" fill="none" stroke-linecap="round" stroke-linejoin="round"/></g>',
        '<path class="ck" d="M198 93l11 11 21-22" stroke="#56d364" stroke-width="5" fill="none" stroke-linecap="round" stroke-linejoin="round"/>',
        # button
        '<g class="pr"><rect x="272" y="78" width="96" height="30" rx="15" fill="#fff"/>'
        '<text class="b1" x="320" y="98" text-anchor="middle" font-size="13" font-weight="700" fill="#0d3b8c">Update now</text>'
        '<text class="b2" x="320" y="98" text-anchor="middle" font-size="13" font-weight="700" fill="#17803a">Up to date</text></g>',
        '<g transform="translate(330 100)"><g class="cu"><path d="M0 0V17l4.5-4.2 3 6.6 3-1.4-3-6.4H14z" fill="#fff" stroke="#0d3b8c" stroke-width="1.4" stroke-linejoin="round"/></g></g>',
    ]
    card("feature-updates.svg", "Updates itself", "One click installs the new version. No admin rights.",
         "An Update now button is clicked; the new version comes from the office server and installs.",
         "".join(art), "".join(css))


# ----------------------------------------------------------------------- steps
def steps():
    T = 8
    xs = (200, 600, 1000)
    cy, r = 112, 58
    steps_ = [("Install", "Run the setup. No admin rights needed."),
              ("Sign in", "Pick the office server, type your name."),
              ("Share", "Open your file, click Share. That's it.")]
    css = [
        ".cn{stroke-dasharray:6 10;animation:cn 1.2s linear infinite}@keyframes cn{to{stroke-dashoffset:-16}}",
        f".tr{{opacity:0;animation:tr {T}s ease-in-out infinite}}",
        "@keyframes tr{0%,8%{opacity:0;transform:translateX(0)}10%{opacity:1}34%{opacity:1;transform:translateX(256px)}36%,100%{opacity:0;transform:translateX(256px)}}",
        ".t2{animation-delay:-" + f"{T * 0.64:.2f}" + "s}",
        f".pu{{opacity:0;animation:pu {T}s ease-out infinite;transform-box:fill-box;transform-origin:center}}",
        "@keyframes pu{0%{opacity:.9;transform:scale(1)}14%,100%{opacity:0;transform:scale(1.45)}}",
        f".s2{{animation-delay:-{T * 0.64:.2f}s}}.s3{{animation-delay:-{T * 0.30:.2f}s}}",
        ".ar1{animation:ar1 1.6s ease-in-out infinite}@keyframes ar1{0%,100%{transform:translateY(-4px)}50%{transform:translateY(4px)}}",
        ".hd{animation:hd 3s ease-in-out infinite}@keyframes hd{0%,100%{transform:translateY(0)}50%{transform:translateY(-3px)}}",
        ".c1{animation:c1 4s ease-in-out infinite}@keyframes c1{0%,15%{transform:translate(0,0)}35%,60%{transform:translate(22px,0)}80%,100%{transform:translate(0,0)}}",
        ".c2{animation:c2 4s ease-in-out infinite}@keyframes c2{0%,10%{transform:translate(0,0)}30%,55%{transform:translate(-22px,-17px)}75%,100%{transform:translate(0,0)}}",
        f".fl{{animation:fl 6s ease-in-out infinite}}.fl2{{animation-delay:-2s}}.fl3{{animation-delay:-4s}}@keyframes fl{{0%,100%{{transform:translateY(0)}}50%{{transform:translateY(-6px)}}}}",
    ]
    art = []
    for i in range(2):
        x1, x2 = xs[i] + r + 14, xs[i + 1] - r - 14
        art.append(f'<path class="cn" d="M{x1} {cy}H{x2}" stroke="#8fb6ff" stroke-opacity=".55" stroke-width="3" stroke-linecap="round"/>')
        art.append(f'<path d="M{x2 - 8} {cy - 8}l8 8-8 8" stroke="#8fb6ff" stroke-opacity=".7" stroke-width="3" fill="none" stroke-linecap="round" stroke-linejoin="round"/>')
        art.append(f'<circle class="tr{" t2" if i else ""}" cx="{x1}" cy="{cy}" r="7" fill="#fff" filter="url(#gw)"/>')
    icons = [
        # 1: download into a tray
        '<g class="ar1"><path d="M0-30V6m-14-14L0 6l14-14" stroke="#fff" stroke-width="6" fill="none" stroke-linecap="round" stroke-linejoin="round"/></g>'
        '<path d="M-26 8V22H26V8" stroke="#bcd3ff" stroke-width="6" fill="none" stroke-linecap="round" stroke-linejoin="round"/>',
        # 2: person with a check badge
        '<g class="hd"><circle cx="0" cy="-12" r="13" fill="#fff"/></g><path d="M-24 28a24 22 0 0 1 48 0z" fill="#fff"/>'
        '<circle cx="21" cy="12" r="11" fill="#2ca02c" stroke="#0b2f72" stroke-width="3"/><path d="M15.5 12l4 4 7-7.5" stroke="#fff" stroke-width="3" fill="none" stroke-linecap="round" stroke-linejoin="round"/>',
        # 3: mini sheet with two cursors
        '<rect x="-30" y="-24" width="60" height="50" rx="5" fill="#fff"/><path d="M-30-10H30M-30 7H30M-8-24V26M14-24V26" stroke="#c9d5ea" stroke-width="1.5"/>'
        '<rect x="-30" y="-24" width="60" height="7" rx="3" fill="#0d3b8c"/>'
        '<g class="c1"><rect x="-29" y="-9" width="21" height="15" rx="1.5" fill="#d62728" fill-opacity=".15" stroke="#d62728" stroke-width="2.6"/></g>'
        '<g class="c2"><rect x="-7" y="8" width="21" height="17" rx="1.5" fill="#2ca02c" fill-opacity=".15" stroke="#2ca02c" stroke-width="2.6"/></g>',
    ]
    for i, (x, (title, sub)) in enumerate(zip(xs, steps_)):
        art.append(f'<circle class="pu s{i + 1}" cx="{x}" cy="{cy}" r="{r}" fill="none" stroke="#8fb6ff" stroke-width="3"/>')
        art.append(f'<g class="fl fl{i + 1}"><circle cx="{x}" cy="{cy}" r="{r}" fill="url(#disc)" stroke="#fff" stroke-opacity=".22" stroke-width="2" filter="url(#ds)"/>')
        art.append(f'<g transform="translate({x} {cy})">{icons[i]}</g>')
        art.append(f'<circle cx="{x - 44}" cy="{cy - 44}" r="17" fill="#fff"/><text x="{x - 44}" y="{cy - 37.5}" text-anchor="middle" font-size="19" font-weight="800" fill="#0d3b8c">{i + 1}</text></g>')
        art.append(f'<text x="{x}" y="222" text-anchor="middle" font-size="30" font-weight="700" fill="#fff">{title}</text>')
        art.append(f'<text x="{x}" y="256" text-anchor="middle" font-size="18" fill="#a9c6f7">{sub}</text>')
    svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 300" width="1200" height="300" role="img" aria-labelledby="t d">
<title id="t">Three steps: Install, Sign in, Share</title>
<desc id="d">1. Install: run the setup, no admin rights needed. 2. Sign in: pick the office server and type your name. 3. Share: open your file from wherever it is saved and click Share; the team edits it with you live.</desc>
<!-- Original artwork by Dexterity Design Services Pvt. Ltd. -->
<defs>
<style>text{{font-family:{FONT}}}{''.join(css)}{REDUCE}</style>
<linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#0f428f"/><stop offset=".6" stop-color="#082866"/><stop offset="1" stop-color="#051a45"/></linearGradient>
<radialGradient id="gl" cx="600" cy="110" r="560" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#3b7bff" stop-opacity=".35"/><stop offset="1" stop-color="#3b7bff" stop-opacity="0"/></radialGradient>
<linearGradient id="disc" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2a63c9"/><stop offset="1" stop-color="#0b2f72"/></linearGradient>
<filter id="ds" x="-30%" y="-30%" width="160%" height="170%"><feDropShadow dx="0" dy="10" stdDeviation="10" flood-color="#020a1f" flood-opacity=".55"/></filter>
<filter id="gw" x="-200%" y="-200%" width="500%" height="500%"><feGaussianBlur stdDeviation="3" result="b"/><feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge></filter>
<clipPath id="cl"><rect width="1200" height="300" rx="24"/></clipPath>
</defs>
<g clip-path="url(#cl)">
<rect width="1200" height="300" fill="url(#bg)"/>
<rect width="1200" height="300" fill="url(#gl)"/>
{''.join(art)}
</g>
<rect x=".75" y=".75" width="1198.5" height="298.5" rx="23.5" fill="none" stroke="#fff" stroke-opacity=".12" stroke-width="1.5"/>
</svg>
"""
    with open(os.path.join(OUT, "steps.svg"), "w", encoding="utf-8", newline="\n") as fh:
        fh.write(svg)
    print("steps.svg", len(svg.encode("utf-8")))


for fn in (live, who, history, versions, excel, updates, steps):
    fn()
