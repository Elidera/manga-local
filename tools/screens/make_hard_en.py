"""Two English pages for tests/hard, drawn after real failures:
- a bigger bold first line in a bubble («HEY!» over «WAIT A MINUTE!», a lone «I»),
  which Windows OCR skipped, so it stayed on the page untranslated;
- captions of three panels side by side at one height, which Windows OCR
  joined into one line across the panels.
Run:  python tools/screens/make_hard_en.py tests/hard   (Pillow, Liberation fonts)
"""
import random
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

BOLD = "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf"
REG = "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf"


def art(d, box, seed, base):
    rnd = random.Random(seed)
    x0, y0, x1, y1 = box
    d.rectangle(box, fill=base)
    for _ in range(90):
        r = rnd.randint(6, 36)
        cx, cy = rnd.randint(x0 + r + 4, x1 - r - 4), rnd.randint(y0 + r + 4, y1 - r - 4)
        c = tuple(max(0, v - rnd.randint(30, 90)) for v in base)
        d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=c, width=2)
    d.rectangle(box, outline=(0, 0, 0), width=5)


def bubble(d, center, lines):
    """lines: (text, font) pairs, centred."""
    cx, cy = center
    sizes = [d.textbbox((0, 0), t, font=f) for t, f in lines]
    hs = [b[3] - b[1] for b in sizes]
    gap = 12
    tw = max(b[2] - b[0] for b in sizes)
    th = sum(hs) + gap * (len(lines) - 1)
    w, h = tw + 90, th + 70
    d.ellipse([cx - w // 2, cy - h // 2, cx + w // 2, cy + h // 2], fill=(255, 255, 255), outline=(0, 0, 0), width=4)
    y = cy - th // 2
    for (t, f), b, lh in zip(lines, sizes, hs):
        d.text((cx - (b[2] - b[0]) // 2 - b[0], y - b[1]), t, font=f, fill=(10, 10, 10))
        y += lh + gap


def main(out):
    out = Path(out)
    big, norm = ImageFont.truetype(BOLD, 46), ImageFont.truetype(BOLD, 26)

    img = Image.new("RGB", (1000, 1400), (255, 255, 255))
    d = ImageDraw.Draw(img)
    art(d, (40, 40, 960, 680), 1, (200, 196, 188))
    art(d, (40, 720, 960, 1360), 2, (190, 196, 200))
    bubble(d, (300, 300), [("HEY!", big), ("WAIT A", norm), ("MINUTE!", norm)])
    bubble(d, (680, 1000), [("I", big), ("WILL DO", norm), ("IT, I PROMISE.", norm)])
    img.save(out / "hard_029_bold_first_line.png")
    (out / "hard_029_bold_first_line.expect").write_text(
        "# Нарисовано tools/screens/make_hard_en.py: первая строка пузыря крупнее и жирнее остальных.\n"
        "lang\ten\nbubbles\t2\nbubble\tHEY! WAIT A MINUTE!\nbubble\tI WILL DO IT, I PROMISE.\n", encoding="utf-8")

    img = Image.new("RGB", (1000, 1400), (255, 255, 255))
    d = ImageDraw.Draw(img)
    cap = ImageFont.truetype(BOLD, 24)
    for i, (x, base, text) in enumerate([(60, (230, 140, 140), ["THE BIRDS,", "INSECTS,"]), (370, (150, 200, 120), ["THE", "ANIMALS."]),
                                         (680, (120, 160, 230), ["...FOR THE", "FRUITS."])]):
        art(d, (x, 120, x + 260, 760), 10 + i, base)
        y = 170
        for t in text:
            d.text((x + 70, y), t, font=cap, fill=(15, 15, 15))
            y += 32
    art(d, (60, 800, 940, 1340), 20, (210, 220, 200))
    d.text((360, 1000), "I'M", font=cap, fill=(15, 15, 15))
    d.text((360, 1032), "THANKFUL FOR ALL", font=cap, fill=(15, 15, 15))
    d.text((360, 1064), "LIFE IN THIS WORLD.", font=cap, fill=(15, 15, 15))
    img.save(out / "hard_030_captions_side_by_side.png")
    (out / "hard_030_captions_side_by_side.expect").write_text(
        "# Нарисовано tools/screens/make_hard_en.py: подписи трёх кадров на одной высоте, «I'M» отдельной строкой.\n"
        "lang\ten\nbubble\tTHE BIRDS, INSECTS,\nbubble\tTHE ANIMALS.\nbubble\t...FOR THE FRUITS.\n"
        "bubble\tI'M THANKFUL FOR ALL LIFE IN THIS WORLD.\n", encoding="utf-8")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "tests/hard")
