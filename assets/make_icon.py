#!/usr/bin/env python3
# Usage: python3 assets/make_icon.py 90 assets/app-icon.png  then  npx tauri icon assets/app-icon.png
# (90 = blue segment on the right, so the wheel reads as a "C")
# Convertino app icon, drawn from scratch (matches the original artwork):
# dark rounded square, 8 wheel segments (40° each, 5° gaps), one blue, light hub.
import sys
from PIL import Image, ImageDraw, ImageFilter
def icon(blue_at_deg, size=1024, ss=4):
    S=size*ss; k=S/1024
    im=Image.new('RGBA',(S,S),(0,0,0,0))
    # soft shadow
    sh=Image.new('RGBA',(S,S),(0,0,0,0)); d=ImageDraw.Draw(sh)
    d.rounded_rectangle([64*k,72*k,960*k,968*k],radius=200*k,fill=(0,0,0,90))
    sh=sh.filter(ImageFilter.GaussianBlur(14*k)); im.alpha_composite(sh)
    d=ImageDraw.Draw(im)
    d.rounded_rectangle([64*k,64*k,960*k,960*k],radius=200*k,fill=(24,28,36,255))
    c=512*k; R=331*k
    for i in range(8):
        mid=blue_at_deg+i*45          # clockwise from 12 o'clock
        col=(0,120,212,255) if i==0 else (70,76,90,255)
        # PIL angles: 0 = 3 o'clock, clockwise
        d.pieslice([c-R,c-R,c+R,c+R],start=mid-90-20,end=mid-90+20,fill=col)
    r=151*k; d.ellipse([c-r,c-r,c+r,c+r],fill=(24,28,36,255))
    r=110*k; d.ellipse([c-r,c-r,c+r,c+r],fill=(235,238,244,255))
    return im.resize((size,size),Image.LANCZOS)
if __name__=='__main__':
    icon(float(sys.argv[1])).save(sys.argv[2])
