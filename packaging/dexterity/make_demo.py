"""Turns the frames captured by tests/screenshots.py into the README's demo animation.

Real app frames only (Vikram's window while three people type one line list); this script just
stages them: the window swings in from a 3D tilt with a light sweep, the frames cross-fade with a
plain-language caption, and the last one (Edit History) holds before the loop. Deterministic: the
same frames give the same animation.

    py -3 packaging/dexterity/make_demo.py [frames dir] [output .webp]
Defaults: tests/out/demo-frames -> docs/readme/demo.webp. Needs Pillow.
"""
from __future__ import annotations

import glob
import math
import os
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

W, H = 1000, 640  # output canvas
SHOT_W = 860  # screenshot width on the canvas when flat
FPS_MS = 40
TOP, BOTTOM = (5, 20, 54), (13, 59, 140)  # brand blues
CAPTIONS = [
    (0.0, "Vikram starts a shared line list on the office server"),
    (0.25, "Anita and Rahul type into it at the same time: each one's cursor carries their name"),
    (0.8, "Every change is signed. Edit History shows who changed what, and when"),
]


def font(size: int) -> ImageFont.ImageFont:
    for name in ("segoeuisb.ttf", "segoeui.ttf", "arial.ttf", "DejaVuSans.ttf"):
        for folder in (os.path.join(os.environ.get("WINDIR", r"C:\Windows"), "Fonts"), "/usr/share/fonts/truetype/dejavu"):
            path = os.path.join(folder, name)
            if os.path.exists(path):
                return ImageFont.truetype(path, size)
    return ImageFont.load_default()


def background() -> Image.Image:
    bg = Image.new("RGB", (W, H))
    d = ImageDraw.Draw(bg)
    for y in range(H):
        t = y / (H - 1)
        d.line([(0, y), (W, y)], fill=tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)))
    # A faint perspective grid on the floor, echoing the icon.
    grid = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    g = ImageDraw.Draw(grid)
    horizon, vx = H * 0.42, W / 2
    for i in range(-12, 13):
        g.line([(vx, horizon), (vx + i * 140, H)], fill=(255, 255, 255, 18), width=1)
    for k in range(1, 9):
        y = horizon + (H - horizon) * (k / 8) ** 1.8
        g.line([(0, y), (W, y)], fill=(255, 255, 255, 18), width=1)
    return Image.alpha_composite(bg.convert("RGBA"), grid)


def rounded(img: Image.Image, r: int) -> Image.Image:
    mask = Image.new("L", img.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, img.width - 1, img.height - 1], r, fill=255)
    out = img.convert("RGBA")
    out.putalpha(mask)
    return out


def solve(a: list[list[float]], b: list[float]) -> list[float]:
    n = len(b)
    m = [row[:] + [b[i]] for i, row in enumerate(a)]
    for c in range(n):
        p = max(range(c, n), key=lambda r: abs(m[r][c]))
        m[c], m[p] = m[p], m[c]
        for r in range(n):
            if r != c:
                f = m[r][c] / m[c][c]
                m[r] = [x - f * y for x, y in zip(m[r], m[c])]
    return [m[i][n] / m[i][i] for i in range(n)]


def coeffs(dst, src):
    """PIL PERSPECTIVE coefficients mapping output points `dst` back to input points `src`."""
    a, b = [], []
    for (x, y), (u, v) in zip(dst, src):
        a.append([x, y, 1, 0, 0, 0, -u * x, -u * y])
        a.append([0, 0, 0, x, y, 1, -v * x, -v * y])
        b += [u, v]
    return solve(a, b)


def quad(w: float, h: float, rx: float, ry: float, scale: float, lift: float):
    """Screen corners of a w x h card rotated rx/ry degrees (3D), seen by a pinhole camera."""
    f = 1600.0
    pts = []
    for x, y in ((-w / 2, -h / 2), (w / 2, -h / 2), (w / 2, h / 2), (-w / 2, h / 2)):
        cy, sy = math.cos(math.radians(rx)), math.sin(math.radians(rx))
        y1, z1 = y * cy, y * sy
        cx_, sx_ = math.cos(math.radians(ry)), math.sin(math.radians(ry))
        x2, z2 = x * cx_ + z1 * sx_, -x * sx_ + z1 * cx_
        k = f / (f + z2) * scale
        pts.append((W / 2 + x2 * k, H / 2 - 34 + lift + y1 * k))
    return pts


def place(bg: Image.Image, card: Image.Image, rx=0.0, ry=0.0, scale=1.0, lift=0.0, sweep=None, alpha=1.0) -> Image.Image:
    pts = quad(card.width, card.height, rx, ry, scale, lift)
    src = [(0, 0), (card.width, 0), (card.width, card.height), (0, card.height)]
    layer = card.transform((W, H), Image.PERSPECTIVE, coeffs(pts, src), Image.BICUBIC)
    shadow = Image.new("L", (W, H), 0)
    ImageDraw.Draw(shadow).polygon([(x, y + 26) for x, y in pts], fill=round(150 * alpha))
    shadow = shadow.filter(ImageFilter.GaussianBlur(22))
    out = bg.copy()
    out.paste((0, 0, 0, 255), (0, 0), shadow)
    if alpha < 1:
        layer.putalpha(layer.getchannel("A").point(lambda p: round(p * alpha)))
    out = Image.alpha_composite(out, layer)
    if sweep is not None:
        band = Image.new("L", (W, H), 0)
        x = -400 + sweep * (W + 800)
        ImageDraw.Draw(band).polygon([(x, 0), (x + 120, 0), (x - 160, H), (x - 280, H)], fill=70)
        band = band.filter(ImageFilter.GaussianBlur(30))
        clip = Image.new("L", (W, H), 0)
        ImageDraw.Draw(clip).polygon(pts, fill=255)
        out.paste((255, 255, 255, 255), (0, 0), ImageChops.multiply(band, clip))
    return out


def caption(img: Image.Image, text: str, alpha: float = 1.0) -> Image.Image:
    f = font(22)
    d = ImageDraw.Draw(img)
    box = d.textbbox((0, 0), text, font=f)
    tw, th = box[2] - box[0], box[3] - box[1]
    pad = 16
    x, y = (W - tw) / 2, H - th - 34
    pill = Image.new("RGBA", (W, H), (0, 0, 0, 0))
    p = ImageDraw.Draw(pill)
    p.rounded_rectangle([x - pad, y - pad + 2, x + tw + pad, y + th + pad], 20, fill=(4, 16, 44, round(205 * alpha)))
    p.text((x, y - box[1]), text, font=f, fill=(255, 255, 255, round(255 * alpha)))
    return Image.alpha_composite(img, pill)


def ease(t: float) -> float:
    return 1 - (1 - t) ** 3


def main() -> int:
    src = sys.argv[1] if len(sys.argv) > 1 else os.path.join("tests", "out", "demo-frames")
    dst = sys.argv[2] if len(sys.argv) > 2 else os.path.join("docs", "readme", "demo.webp")
    files = sorted(glob.glob(os.path.join(src, "frame-*.png")))
    if len(files) < 3:
        print(f"need at least 3 frames in {src}, found {len(files)}")
        return 1
    shots = []
    for path in files:
        im = Image.open(path).convert("RGB")
        im = im.resize((SHOT_W, round(im.height * SHOT_W / im.width)), Image.LANCZOS)
        shots.append(rounded(im, 14))
    bg = background()
    frames: list[Image.Image] = []
    durations: list[int] = []

    def emit(img: Image.Image, ms: int, text: str | None = None, text_alpha: float = 1.0):
        if text:
            img = caption(img, text, text_alpha)
        frames.append(img.convert("RGB"))
        durations.append(ms)

    def text_for(i: int) -> str:
        pos = i / max(1, len(shots) - 1)
        return [t for at, t in CAPTIONS if pos >= at][-1]

    # Swing in: tilted and lower, flattening while a light sweep crosses the window.
    steps = 22
    for k in range(steps + 1):
        t = ease(k / steps)
        emit(place(bg, shots[0], rx=28 * (1 - t), ry=-24 * (1 - t), scale=0.78 + 0.22 * t, lift=60 * (1 - t), sweep=k / steps, alpha=min(1, 0.2 + k / 6)), FPS_MS, text_for(0), min(1, k / steps * 2))
    durations[-1] = 900
    # The real frames, cross-faded.
    flat = [place(bg, s) for s in shots]
    for i in range(1, len(shots)):
        for k in range(1, 5):
            emit(Image.blend(flat[i - 1], flat[i], k / 4), FPS_MS, text_for(i))
        durations[-1] = 2600 if i == len(shots) - 1 else 850
    # Swing out to the opening pose so the loop joins up.
    for k in range(1, 13):
        t = ease(k / 12)
        emit(place(bg, shots[-1], rx=28 * t, ry=-24 * t, scale=1 - 0.22 * t, lift=60 * t, alpha=1 - 0.8 * t), FPS_MS)
    os.makedirs(os.path.dirname(os.path.abspath(dst)), exist_ok=True)
    frames[0].save(dst, save_all=True, append_images=frames[1:], duration=durations, loop=0, quality=72, method=6)
    print(f"{dst}: {len(frames)} frames, {sum(durations) / 1000:.1f} s, {os.path.getsize(dst) / 1e6:.2f} MB")
    return 0


if __name__ == "__main__":
    sys.exit(main())
