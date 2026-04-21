"""Generate all icons for the power-controle app using Pillow."""

from PIL import Image, ImageDraw, ImageFont
import math
import os

TRAY_SIZE = 32
APP_SIZES = [32, 128, 256]
OUT = os.path.join(os.path.dirname(__file__), "src-tauri", "icons")
os.makedirs(OUT, exist_ok=True)

# ── helpers ──────────────────────────────────────────────────────────

def new(size, bg=(0, 0, 0, 0)):
    img = Image.new("RGBA", (size, size), bg)
    return img, ImageDraw.Draw(img)


def save(img, name):
    path = os.path.join(OUT, name)
    img.save(path, "PNG")
    print(f"  {name} ({img.size[0]}x{img.size[1]})")


# ── tray icons (32×32, white on transparent) ─────────────────────────

def draw_lightning(size=TRAY_SIZE):
    """Lightning bolt — High Performance."""
    img, d = new(size)
    # simple zigzag bolt
    s = size
    points = [
        (s*0.55, s*0.05),
        (s*0.25, s*0.45),
        (s*0.50, s*0.45),
        (s*0.35, s*0.95),
        (s*0.75, s*0.40),
        (s*0.50, s*0.40),
        (s*0.55, s*0.05),
    ]
    d.polygon(points, fill=(255, 200, 50, 255))
    # subtle outline
    d.polygon(points, outline=(200, 150, 0, 255), width=1)
    return img


def draw_leaf(size=TRAY_SIZE):
    """Leaf — Power Saver."""
    img, d = new(size)
    s = size
    # leaf body (ellipse rotated ~45°)
    leaf = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    ld = ImageDraw.Draw(leaf)
    # draw an ellipse then rotate
    pad = int(s * 0.15)
    ld.ellipse([pad, int(s*0.25), s-pad, int(s*0.85)], fill=(80, 190, 80, 255))
    leaf = leaf.rotate(-35, resample=Image.BICUBIC, center=(s//2, s//2))
    img = Image.alpha_composite(img, leaf)
    d = ImageDraw.Draw(img)
    # leaf vein (center line)
    cx, cy = s*0.48, s*0.50
    d.line([(cx, cy-s*0.22), (cx, cy+s*0.22)], fill=(40, 130, 40, 230), width=max(1, s//16))
    # small stem
    d.line([(cx, cy+s*0.22), (cx+s*0.12, cy+s*0.36)], fill=(40, 130, 40, 230), width=max(1, s//16))
    return img


def draw_balanced(size=TRAY_SIZE):
    """Half-circle / yin-yang style — Balanced."""
    img, d = new(size)
    s = size
    pad = int(s * 0.12)
    # full circle outline
    d.ellipse([pad, pad, s-pad, s-pad], outline=(100, 160, 255, 255), width=max(2, s//12))
    # left half filled
    d.pieslice([pad, pad, s-pad, s-pad], start=90, end=270, fill=(100, 160, 255, 255))
    return img


# ── app icon (power-button symbol) ──────────────────────────────────

def draw_app_icon(size):
    """Power button symbol — circle + top line."""
    img, d = new(size, bg=(30, 30, 30, 255))
    s = size
    pad = int(s * 0.15)
    lw = max(2, s // 10)

    # outer ring (arc, gap at top)
    d.arc([pad, pad, s-pad, s-pad], start=40, end=320, fill=(100, 200, 255, 255), width=lw)
    # vertical line at top center
    cx = s // 2
    d.line([(cx, int(s*0.12)), (cx, int(s*0.48))], fill=(100, 200, 255, 255), width=lw)

    return img


# ── generate everything ─────────────────────────────────────────────

print("Generating tray mode icons (32×32):")
save(draw_lightning(), "tray_high_performance.png")
save(draw_leaf(), "tray_power_saver.png")
save(draw_balanced(), "tray_balanced.png")

print("\nGenerating app icons:")
for sz in APP_SIZES:
    save(draw_app_icon(sz), f"{sz}x{sz}.png")

# 128@2x = 256
save(draw_app_icon(256), "128x128@2x.png")

# icon.png (512 for general use)
save(draw_app_icon(512), "icon.png")

# icon.ico (Windows multi-size)
ico_imgs = [draw_app_icon(s) for s in [16, 32, 48, 256]]
ico_path = os.path.join(OUT, "icon.ico")
ico_imgs[0].save(ico_path, format="ICO", sizes=[(s, s) for s in [16, 32, 48, 256]],
                  append_images=ico_imgs[1:])
print(f"  icon.ico (multi-size)")

print("\nDone!")
