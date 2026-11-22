"""Rebuild bundle assets from committed, exact-size Fold PNGs.

Requires Python Pillow (an asset-authoring tool only).
16/32 PNGs are copied verbatim, never resized from a larger representation.
"""
from pathlib import Path
import struct
from PIL import Image

root = Path(__file__).resolve().parent.parent
source = root / 'public/app-icons/fold'
dest = root / 'src-tauri/icons'
def image(size):
    exact = source / f'{size}.png'
    if exact.exists():
        return exact.read_bytes()
    import io
    out = io.BytesIO()
    Image.open(source/'1024.png').resize((size,size),Image.Resampling.LANCZOS).save(out,format='PNG')
    return out.getvalue()
for size, name in [(16,'16x16.png'),(32,'32x32.png'),(64,'64x64.png'),(128,'128x128.png'),(256,'128x128@2x.png'),(512,'icon.png'),(1024,'source.png')]:
    (dest/name).write_bytes(image(size))
# PNG-backed ICNS avoids iconutil's conversion of tiny PNGs into ARGB chunks,
# which changes edge RGB values. Include 1x and Retina representations.
chunks=[]
for tag,size in [(b'icp4',16),(b'icp5',32),(b'icp6',64),(b'ic07',128),
                 (b'ic08',256),(b'ic09',512),(b'ic10',1024),
                 (b'ic11',32),(b'ic12',64),(b'ic13',256),(b'ic14',512)]:
    data=image(size);chunks.append(tag+struct.pack('>I',len(data)+8)+data)
body=b''.join(chunks)
(dest/'icon.icns').write_bytes(b'icns'+struct.pack('>I',len(body)+8)+body)
# PNG-backed ICO representations preserve the exact tiny renders too.
sizes=[16,32,64,128,256];parts=[image(s) for s in sizes];offset=6+16*len(parts)
entries=[]
for size,data in zip(sizes,parts):
    entries.append(struct.pack('<BBBBHHII',size%256,size%256,0,0,1,32,len(data),offset));offset+=len(data)
(dest/'icon.ico').write_bytes(struct.pack('<HHH',0,1,len(parts))+b''.join(entries)+b''.join(parts))
(root/'src/assets/brand/hollow-mark-96.png').write_bytes(image(96))
(root/'public/hollow-16.png').write_bytes(image(16))
(root/'public/hollow-32.png').write_bytes(image(32))
(root/'public/hollow.svg').write_bytes((source/'source.svg').read_bytes())
print('Rebuilt Fold bundle, sidebar and favicon assets from committed representations.')
