"""The scroller's big font and its text.

    python3 font.py [out dir]

The font is Noto Sans Black (SIL Open Font License) rendered at CAP_HEIGHT
pixels in a ROWS-row cell: each glyph is a run of columns, each column a
32-bit mask (bit r is row r, from the top), with a blank column either side
for the demo's outline. Writes font.png in the out dir to look at.

TEXT is the scroll text: upper case, digits and the punctuation in CHARS.
"""

import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
FONT_FILE = "/usr/share/fonts/truetype/noto/NotoSans-Black.ttf"
ROWS = 32
CAP_HEIGHT = 27
BASELINE = 29
SPACE = 12
GAP = 2
CHARS = " ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.,!?-'/:&*+()"

TEXT = (
    "        INTO THE GRID ...   A MEGA65 DEMO, WRITTEN IN MENSCH: A LANGUAGE MADE "
    "FOR THE 65XX FAMILY, COMPILED FOR THE 45GS02 AT 40 MHZ ...   EVERY PIXEL AND "
    "EVERY NOTE YOU SEE AND HEAR IS MADE LIVE, FRAME BY FRAME ...   "
    "FOR JUAN ATKINS, WHO TAUGHT THE MACHINES TO DANCE AND GAVE DETROIT ITS FUTURE "
    "SOUND ...   ELECTRO FOREVER ...   "
    "GREETINGS TO EVERYONE STILL PUSHING OLD SILICON PAST ITS LIMITS, AND TO ALL "
    "WHO BUILD THINGS JUST TO SEE THEM MOVE ...   "
    "THE GRID IS WAITING ...   STEP THROUGH THE GLASS ...          "
)


def glyphs():
    """Per character in CHARS, its columns as 32-bit masks."""
    font = ImageFont.truetype(FONT_FILE, 100)
    # Size the font so its capitals are CAP_HEIGHT pixels.
    top, bottom = font.getbbox("H")[1], font.getbbox("H")[3]
    size = round(100 * CAP_HEIGHT / (bottom - top))
    font = ImageFont.truetype(FONT_FILE, size)
    h_top = font.getbbox("H")[1]
    out = []
    for ch in CHARS:
        if ch == " ":
            out.append([0] * SPACE)
            continue
        box = font.getbbox(ch)
        width = box[2] - box[0] + 2
        img = Image.new("L", (width, ROWS), 0)
        ImageDraw.Draw(img).text((1 - box[0], BASELINE - CAP_HEIGHT - h_top), ch, font=font, fill=255)
        pixels = np.array(img) >= 128
        columns = [0] * GAP
        for x in range(width):
            mask = 0
            for y in range(ROWS):
                if pixels[y, x]:
                    mask |= 1 << y
            columns.append(mask)
        # Trim the blank columns the bounding box left, keeping GAP each side.
        while len(columns) > GAP and columns[GAP] == 0 and columns[GAP + 1] == 0:
            del columns[GAP]
        while columns and columns[-1] == 0:
            columns.pop()
        out.append(columns + [0] * GAP)
    return out


def text_indices():
    missing = sorted(set(TEXT) - set(CHARS))
    if missing:
        raise SystemExit(f"scroll text uses characters the font lacks: {missing}")
    return [CHARS.index(ch) for ch in TEXT]


def main():
    out_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "target" / "into-the-grid"
    out_dir.mkdir(parents=True, exist_ok=True)
    columns = [c for g in glyphs() for c in g]
    sheet = np.array([[(c >> y) & 1 for c in columns] for y in range(ROWS)], dtype=np.uint8) * 255
    Image.fromarray(sheet).resize((len(columns) * 2, ROWS * 2), Image.NEAREST).save(out_dir / "font.png")
    widths = sum(len(glyphs()[i]) for i in text_indices())
    print(f"font: {len(CHARS)} glyphs, {len(columns)} columns; text {len(TEXT)} characters, {widths} columns")


if __name__ == "__main__":
    main()
