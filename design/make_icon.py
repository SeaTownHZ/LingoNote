from PIL import Image, ImageDraw, ImageFont, ImageFilter
import os

OUT = r"D:\ClipTonge\design"
os.makedirs(OUT, exist_ok=True)

S = 2048
TOP = (14, 59, 58)
BOTTOM = (30, 107, 98)
NOTE = (255, 248, 231, 255)
FOLD = (234, 217, 184, 255)
INK = (36, 74, 70, 210)
CORAL = (242, 163, 101, 255)
WHITE = (255, 248, 231, 255)


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def rounded_mask(size, radius):
    mask = Image.new("L", size, 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, size[0] - 1, size[1] - 1], radius=radius, fill=255
    )
    return mask


def shadow_of(img, base_size, pos, offset=(0, 56), blur=52, alpha=95):
    sh = Image.new("RGBA", base_size, (0, 0, 0, 0))
    a = img.getchannel("A").point(lambda v: int(v * alpha / 255))
    sh.paste((0, 20, 18, 255), (pos[0] + offset[0], pos[1] + offset[1]), a)
    return sh.filter(ImageFilter.GaussianBlur(blur))


def paste_center(base, img, center):
    x = int(center[0] - img.width / 2)
    y = int(center[1] - img.height / 2)
    base.alpha_composite(shadow_of(img, base.size, (x, y)))
    base.alpha_composite(img, (x, y))
    return x, y


# Background
bg = Image.new("RGBA", (S, S), (0, 0, 0, 0))
grad = Image.new("RGB", (S, S))
gd = ImageDraw.Draw(grad)
for y in range(S):
    t = y / (S - 1)
    t = t * t * (3 - 2 * t)
    gd.line([(0, y), (S, y)], fill=lerp(TOP, BOTTOM, t))
bg.paste(grad, (0, 0), rounded_mask((S, S), 440))

# Note
NW, NH = 1040, 1200
note_canvas = Image.new("RGBA", (1600, 1700), (0, 0, 0, 0))
nd = ImageDraw.Draw(note_canvas)
nx, ny = (1600 - NW) // 2, (1700 - NH) // 2
nd.rounded_rectangle(
    [nx, ny, nx + NW - 1, ny + NH - 1], radius=100, fill=NOTE
)
fold = 220
nd.polygon(
    [(nx + NW - fold, ny), (nx + NW - 1, ny), (nx + NW - 1, ny + fold)],
    fill=FOLD,
)
nd.line(
    [(nx + NW - fold, ny), (nx + NW - 1, ny + fold)],
    fill=(214, 194, 163, 255),
    width=6,
)
lines = [(620, 340), (500, 460), (560, 580), (430, 700)]
for w, y in lines:
    nd.rounded_rectangle(
        [nx + 140, ny + y, nx + 140 + w, ny + y + 30], radius=15, fill=INK
    )
note_rot = note_canvas.rotate(-6, resample=Image.BICUBIC, expand=True)
note_rot = note_rot.crop(note_rot.getbbox())
paste_center(bg, note_rot, (880, 930))

# Bubble
BW, BH = 680, 500
bub = Image.new("RGBA", (BW, BH), (0, 0, 0, 0))
bd = ImageDraw.Draw(bub)
bd.rounded_rectangle([0, 0, BW - 1, 380], radius=90, fill=CORAL)
bd.polygon([(100, 370), (250, 370), (150, 500)], fill=CORAL)

font = None
for p in [
    r"C:\Windows\Fonts\msyhbd.ttc",
    r"C:\Windows\Fonts\msyh.ttc",
    r"C:\Windows\Fonts\simhei.ttf",
    r"C:\Windows\Fonts\arialbd.ttf",
]:
    if os.path.exists(p):
        try:
            font = ImageFont.truetype(p, 220, index=0)
            break
        except Exception:
            pass
if font is None:
    font = ImageFont.load_default()

text = "文"
if font.getname()[0].lower().startswith("arial"):
    text = "A"
bbox = bd.textbbox((0, 0), text, font=font)
tw, th = bbox[2] - bbox[0], bbox[3] - bbox[1]
bd.text(
    ((BW - tw) / 2 - bbox[0], (380 - th) / 2 - bbox[1] - 8),
    text,
    font=font,
    fill=WHITE,
)
bub_rot = bub.rotate(3, resample=Image.BICUBIC, expand=True)
bub_rot = bub_rot.crop(bub_rot.getbbox())
paste_center(bg, bub_rot, (1370, 1390))

icon = bg.resize((1024, 1024), Image.LANCZOS)
icon.save(os.path.join(OUT, "lingonote-1024.png"))
icon.resize((512, 512), Image.LANCZOS).save(os.path.join(OUT, "lingonote-512.png"))
icon.resize((256, 256), Image.LANCZOS).save(os.path.join(OUT, "lingonote-256.png"))
icon.save(
    os.path.join(OUT, "lingonote.ico"),
    sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
)


def make_tray(color, size=176):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    k = size / 44

    def P(v):
        return int(round(v * k))

    d.rounded_rectangle(
        [P(5), P(4), P(30), P(38)],
        radius=P(4),
        outline=color,
        width=P(3),
    )
    d.line([(P(10), P(13)), (P(24), P(13))], fill=color, width=P(3))
    d.line([(P(10), P(20)), (P(22), P(20))], fill=color, width=P(3))
    d.line([(P(10), P(27)), (P(20), P(27))], fill=color, width=P(3))
    d.rounded_rectangle(
        [P(22), P(23), P(40), P(36)], radius=P(3), fill=color
    )
    d.polygon(
        [(P(26), P(35)), (P(33), P(35)), (P(28), P(42))], fill=color
    )
    return img.resize((44, 44), Image.LANCZOS)


tray_black = make_tray((0, 0, 0, 255))
tray_white = make_tray((255, 255, 255, 255))
tray_black.save(os.path.join(OUT, "tray-black-44.png"))
tray_white.save(os.path.join(OUT, "tray-white-44.png"))
tray_black.resize((22, 22), Image.LANCZOS).save(os.path.join(OUT, "tray-black-22.png"))
tray_white.resize((22, 22), Image.LANCZOS).save(os.path.join(OUT, "tray-white-22.png"))

# Preview sheet
preview = Image.new("RGBA", (1500, 760), (246, 243, 236, 255))
pd = ImageDraw.Draw(preview)
preview.alpha_composite(icon.resize((520, 520), Image.LANCZOS), (40, 70))
label_font = ImageFont.truetype(r"C:\Windows\Fonts\msyh.ttc", 26)
pd.text((40, 610), "LingoNote / 语签", font=label_font, fill=(27, 42, 65, 255))
pd.text((40, 650), "复制，按热键，读译文。", font=ImageFont.truetype(r"C:\Windows\Fonts\msyh.ttc", 22), fill=(70, 90, 110, 255))

# light strip
light = Image.new("RGBA", (880, 230), (255, 255, 255, 255))
ld = ImageDraw.Draw(light)
x = 40
for sz in (16, 24, 32, 48, 64, 128):
    im = icon.resize((sz, sz), Image.LANCZOS)
    light.alpha_composite(im, (x, 50 + (128 - sz) // 2))
    ld.text((x, 190), str(sz), font=ImageFont.truetype(r"C:\Windows\Fonts\msyh.ttc", 18), fill=(60, 70, 80, 255))
    x += sz + 45
preview.alpha_composite(light, (600, 50))

# dark strip
dark = Image.new("RGBA", (880, 230), (26, 32, 38, 255))
dd = ImageDraw.Draw(dark)
x = 40
for sz in (16, 24, 32, 48, 64, 128):
    im = icon.resize((sz, sz), Image.LANCZOS)
    dark.alpha_composite(im, (x, 50 + (128 - sz) // 2))
    dd.text((x, 190), str(sz), font=ImageFont.truetype(r"C:\Windows\Fonts\msyh.ttc", 18), fill=(220, 225, 230, 255))
    x += sz + 45
preview.alpha_composite(dark, (600, 300))

# tray strip
tray_strip = Image.new("RGBA", (880, 170), (238, 238, 238, 255))
td = ImageDraw.Draw(tray_strip)
td.text((40, 18), "托盘图标 44 / 22（浅色）", font=ImageFont.truetype(r"C:\Windows\Fonts\msyh.ttc", 20), fill=(50, 60, 70, 255))
tray_strip.alpha_composite(tray_black, (40, 60))
tray_strip.alpha_composite(tray_black.resize((22, 22), Image.LANCZOS), (120, 71))
tray_strip.alpha_composite(tray_white, (220, 60))
tray_strip.alpha_composite(tray_white.resize((22, 22), Image.LANCZOS), (300, 71))
# dark tray strip
tray_dark = Image.new("RGBA", (880, 170), (26, 32, 38, 255))
td2 = ImageDraw.Draw(tray_dark)
td2.text((40, 18), "托盘图标 44 / 22（深色）", font=ImageFont.truetype(r"C:\Windows\Fonts\msyh.ttc", 20), fill=(225, 230, 235, 255))
tray_dark.alpha_composite(tray_white, (40, 60))
tray_dark.alpha_composite(tray_white.resize((22, 22), Image.LANCZOS), (120, 71))
preview.alpha_composite(tray_strip, (600, 550))

preview.save(os.path.join(OUT, "icon-preview.png"))
print("done")
