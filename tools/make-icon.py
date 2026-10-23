# Draws assets/icon.ico, the exe icon: the same amber rounded square with a dark "!" that the app
# draws for its tray icon (src/win/gfx.rs, make_icon), at every size Windows asks for.
# Needs Pillow. Run from the repo root: python tools/make-icon.py
from PIL import Image, ImageDraw

BIG = 1024  # drawn large on the 32-unit grid of make_icon, then scaled down per size
u = BIG / 32
img = Image.new('RGBA', (BIG, BIG), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
d.rounded_rectangle([2 * u, 2 * u, 30 * u, 30 * u], radius=7 * u, fill=(0xFF, 0xBA, 0x00, 255))
d.rectangle([14 * u, 7 * u, 18 * u, 20 * u], fill=(0x1E, 0x1E, 0x1E, 255))
d.rectangle([14 * u, 22 * u, 18 * u, 26 * u], fill=(0x1E, 0x1E, 0x1E, 255))
sizes = [16, 20, 24, 32, 40, 48, 64, 256]
img.save('assets/icon.ico', sizes=[(s, s) for s in sizes])
img.resize((256, 256), Image.LANCZOS).save('assets/icon.png')
print('wrote assets/icon.ico and assets/icon.png')
