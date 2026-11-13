"""Generate ClipWheel's original vector logo and multi-resolution Windows icon."""
from pathlib import Path
from math import cos, sin, radians
from PIL import Image, ImageDraw
import io, struct
OUT=Path(__file__).resolve().parents[1]/'assets'
OUT.mkdir(exist_ok=True)
BASE='#1e1e2e'; LAVENDER='#b4befe'; TEXT='#cdd6f4'; MUTED='#585b70'
# Geometric source of truth: segmented wheel surrounding a clipboard.
svg=['<svg xmlns="http://www.w3.org/2000/svg" width="512" height="512" viewBox="0 0 512 512" role="img" aria-label="ClipWheel logo">',
     f'<rect x="16" y="16" width="480" height="480" rx="112" fill="{BASE}"/>']
for i in range(8):
    a,b=-108+i*45,-72+i*45
    x1,y1=256+174*cos(radians(a)),256+174*sin(radians(a))
    x2,y2=256+174*cos(radians(b)),256+174*sin(radians(b))
    tint=LAVENDER if i in (0,1,2) else MUTED
    svg.append(f'<path d="M{x1:.3f},{y1:.3f} A174,174 0 0 1 {x2:.3f},{y2:.3f}" fill="none" stroke="{tint}" stroke-width="20" stroke-linecap="round"/>')
svg += [f'<rect x="185" y="175" width="142" height="184" rx="22" fill="{BASE}" stroke="{TEXT}" stroke-width="18"/>',
        f'<rect x="221" y="153" width="70" height="42" rx="12" fill="{BASE}" stroke="{LAVENDER}" stroke-width="16"/>',
        f'<path d="M219 242h74 M219 280h74 M219 318h46" fill="none" stroke="{TEXT}" stroke-width="14" stroke-linecap="round"/>','</svg>']
(OUT/'clipwheel-logo.svg').write_text('\n'.join(svg),encoding='utf-8')
def draw_logo(size):
    # Explicit small-size geometry preserves legibility at tray scale.
    ss=4; canvas=Image.new('RGBA',(size*ss,size*ss));d=ImageDraw.Draw(canvas)
    k=size*ss/512
    box=lambda values: tuple(v*k for v in values)
    d.rounded_rectangle(box((16,16,496,496)),radius=112*k,fill=BASE)
    for i in range(8):
        a,b=-108+i*45,-72+i*45
        tint=LAVENDER if i in (0,1,2) else MUTED
        d.arc(box((82,82,430,430)),a,b,fill=tint,width=max(1,round(20*k)))
    d.rounded_rectangle(box((185,175,327,359)),radius=22*k,fill=BASE,outline=TEXT,width=max(1,round((24 if size<=24 else 18)*k)))
    d.rounded_rectangle(box((221,153,291,195)),radius=12*k,fill=BASE,outline=LAVENDER,width=max(1,round(16*k)))
    if size>=32:
        for y,end in [(242,293),(280,293),(318,265)]:d.line(box((219,y,end,y)),fill=TEXT,width=max(1,round(14*k)))
    return canvas.resize((size,size),Image.Resampling.LANCZOS)
draw_logo(512).save(OUT/'clipwheel-logo.png')
sizes=[16,20,24,32,48,64,128,256]
images=[]
for size in sizes:
    image=draw_logo(size)
    image.save(OUT/f'clipwheel-{size}.png')
    b=io.BytesIO();image.save(b,format='PNG');images.append(b.getvalue())
header=struct.pack('<HHH',0,1,len(sizes));offset=6+16*len(sizes);directory=b''
for size,blob in zip(sizes,images):
    directory+=struct.pack('<BBBBHHII',size%256,size%256,0,0,1,32,len(blob),offset);offset+=len(blob)
(OUT/'clipwheel.ico').write_bytes(header+directory+b''.join(images))
print('Created SVG, PNG and 8-size ICO in',OUT)
