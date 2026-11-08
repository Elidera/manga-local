"""Draw synthetic manga / manhwa pages for the screenshot tests (tests/screens).

Original pictures: grey panels with noise as "artwork", white speech bubbles
with Korean or Japanese text, a Latin chapter caption that must be ignored.
Run:  python tools/screens/make_pages.py tests/screens
Needs Pillow and fonts with Hangul and kana (WenQuanYi Zen Hei, IPAGothic).
"""
import random
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

KO_FONT = "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc"
JA_FONT = "/usr/share/fonts/opentype/ipafont-gothic/ipag.ttf"
LATIN_FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"


def panel(d, box, seed):
    rnd = random.Random(seed)
    x0, y0, x1, y1 = box
    for y in range(y0, y1, 4):
        g = 150 + int(60 * (y - y0) / max(1, y1 - y0))
        d.rectangle([x0, y, x1, y + 4], fill=(g, g - 8, g - 16))
    for _ in range(140):
        r = rnd.randint(6, 40)
        cx, cy = rnd.randint(x0 + r + 4, x1 - r - 4), rnd.randint(y0 + r + 4, y1 - r - 4)
        c = rnd.randint(60, 200)
        d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=(c, c, c), width=2)
    d.rectangle(box, outline=(0, 0, 0), width=5)


def bubble(d, center, lines, font, pad=(46, 34)):
    cx, cy = center
    sizes = [d.textbbox((0, 0), l, font=font) for l in lines]
    lh = max(b[3] - b[1] for b in sizes)
    gap = int(lh * 0.35)
    tw = max(b[2] - b[0] for b in sizes)
    th = lh * len(lines) + gap * (len(lines) - 1)
    w, h = tw + 2 * pad[0], th + 2 * pad[1]
    d.ellipse([cx - w // 2, cy - h // 2, cx + w // 2, cy + h // 2], fill=(255, 255, 255), outline=(0, 0, 0), width=4)
    y = cy - th // 2
    for l, b in zip(lines, sizes):
        lw = b[2] - b[0]
        d.text((cx - lw // 2 - b[0], y - b[1]), l, font=font, fill=(10, 10, 10))
        y += lh + gap


def page(bubbles, font_path, caption, seed):
    img = Image.new("RGB", (1000, 1400), (255, 255, 255))
    d = ImageDraw.Draw(img)
    panel(d, (40, 40, 960, 640), seed)
    panel(d, (40, 700, 960, 1360), seed + 1)
    font = ImageFont.truetype(font_path, 34)
    for center, lines in bubbles:
        bubble(d, center, lines, font)
    d.text((60, 1300), caption, font=ImageFont.truetype(LATIN_FONT, 30), fill=(255, 255, 255), stroke_width=3, stroke_fill=(0, 0, 0))
    return img


def main(out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    ko = [
        ((300, 170), ["뭐야, 벌써 왔어?"]),
        ((640, 450), ["늦어서 미안해.", "길이 너무 막혔어."]),
        ((330, 860), ["괜찮아. 나도", "방금 도착했어."]),
        ((640, 1140), ["오늘은 내가 살게.", "먹고 싶은 거 다 시켜."]),
    ]
    page(ko, KO_FONT, "Chapter 12", 1).save(out / "manhwa_ko_cafe.png")
    # Japanese reads right to left: the right bubble of the first row comes first.
    ja = [
        ((700, 180), ["先輩、遅いですよ！"]),
        ((270, 420), ["悪い悪い、", "電車が止まっててさ。"]),
        ((640, 880), ["今日こそ告白するって", "決めたんだ。"]),
        ((330, 1160), ["大丈夫？", "顔、真っ赤だよ。"]),
    ]
    page(ja, JA_FONT, "Episode 3", 7).save(out / "manga_ja_station.png")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "tests/screens")
