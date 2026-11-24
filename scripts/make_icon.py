# /// script
# requires-python = ">=3.11"
# dependencies = ["pillow"]
# ///
"""Builds assets/icon.png (1024x1024, transparent) from assets/icon-source.png.

The source is a small rounded-square image on a white background. We crop
inside its white corner fringe, upscale, re-mask to a clean rounded square,
and place it on the standard macOS icon canvas (824px art, 100px margin) with
a soft drop shadow.

    uv run scripts/make_icon.py
"""
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ASSETS = Path(__file__).resolve().parent.parent / "assets"
CANVAS, ART = 1024, 824
INSET = 5            # px of the source to trim (white antialiasing fringe)
RADIUS = 0.225       # corner radius as a fraction of the art size
BLUR = 7            # px, applied after upscaling
SS = 4               # supersampling for the antialiased mask


def rounded_mask(size: int) -> Image.Image:
    big = Image.new("L", (size * SS, size * SS), 0)
    r = int(size * SS * RADIUS)
    ImageDraw.Draw(big).rounded_rectangle((0, 0, size * SS - 1, size * SS - 1), r, fill=255)
    return big.resize((size, size), Image.LANCZOS)


def main() -> None:
    src = Image.open(ASSETS / "icon-source.png").convert("RGB")
    w, h = src.size
    side = min(w, h) - 2 * INSET
    left, top = (w - side) // 2, (h - side) // 2
    art = src.crop((left, top, left + side, top + side)).resize((ART, ART), Image.BICUBIC)
    # The art is a smooth glow; a light blur hides the source's compression blocks.
    art = art.filter(ImageFilter.GaussianBlur(BLUR))
    art.putalpha(rounded_mask(ART))

    offset = (CANVAS - ART) // 2
    canvas = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))

    shadow = Image.new("RGBA", (CANVAS, CANVAS), (0, 0, 0, 0))
    shadow_mask = Image.new("L", (CANVAS, CANVAS), 0)
    shadow_mask.paste(art.getchannel("A"), (offset, offset + 12))
    shadow_mask = shadow_mask.filter(ImageFilter.GaussianBlur(16)).point(lambda v: v * 0.35)
    shadow.putalpha(shadow_mask)
    canvas = Image.alpha_composite(canvas, shadow)
    canvas.alpha_composite(art, (offset, offset))

    out = ASSETS / "icon.png"
    canvas.save(out)
    print(f"wrote {out} ({CANVAS}x{CANVAS})")


if __name__ == "__main__":
    main()
