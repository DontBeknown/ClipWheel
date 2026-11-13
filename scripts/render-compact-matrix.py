from pathlib import Path
from PIL import Image, ImageDraw
import numpy as np
folder=Path('docs/verification/compact')
for p in folder.glob('*.bgra'):
    w,h=map(int,p.stem.split('-')[-1].split('x'))
    a=np.frombuffer(p.read_bytes(),dtype=np.uint8).reshape(h,w,4).copy()
    alpha=a[:,:,3:4].astype(float)
    rgb=np.minimum(255,a[:,:,[2,1,0]].astype(float)*255/np.maximum(alpha,1)).astype(np.uint8)
    img=Image.fromarray(np.concatenate([rgb,a[:,:,3:4]],axis=2),'RGBA')
    img.save(p.with_suffix('.png'))
    p.unlink()  # Raw GPU capture is temporary; retain the compact PNG.
    for theme,bg in [('light',(235,238,243,255)),('dark',(24,29,40,255)),('busy',(75,94,120,255))]:
        back=Image.new('RGBA',(w,h),bg)
        if theme=='busy':
            d=ImageDraw.Draw(back)
            for y in range(0,h,24):
                for x in range(0,w,90):
                    d.rectangle((x,y,x+42,y+12),fill=(190,120,65,255))
        back.alpha_composite(img)
        back.convert('RGB').save(folder/(p.stem+'-'+theme+'.jpg'))
# Contact sheets at native 100% scale, and all DPI renders normalized for comparison.
for name in ['eight-overflow','eight-fit','empty','single','files','image','tooltip-short']:
    sheet=Image.new('RGB',(560,332*3),'white')
    for i,theme in enumerate(['light','dark','busy']):
        sheet.paste(Image.open(folder/f'{name}-100-560x332-{theme}.jpg'),(0,i*332))
    sheet.save(folder/f'{name}-backgrounds.jpg')
