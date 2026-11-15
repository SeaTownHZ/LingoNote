from PIL import Image
import os

src = Image.open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "lingonote-1024.png"))
out = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "src-tauri", "icons"))
os.makedirs(out, exist_ok=True)

for size, name in [
    (32, "32x32.png"),
    (128, "128x128.png"),
    (256, "128x128@2x.png"),
    (512, "icon.png"),
]:
    src.resize((size, size), Image.LANCZOS).save(os.path.join(out, name))

src.save(
    os.path.join(out, "icon.ico"),
    sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)],
)
print("icons written")
