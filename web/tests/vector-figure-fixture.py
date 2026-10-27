#!/usr/bin/env python3
"""Original vector-only PDF fixture: edges, colour fills and rotated labels.
No image XObjects or external paper content. Also includes ordinary prose and
rotated-watermark negative controls. Run to regenerate fixtures/vector-figure.pdf.
"""
from pathlib import Path

objects = []
def obj(body):
    objects.append(body.encode('ascii'))
    return len(objects)

obj('<< /Type /Catalog /Pages 2 0 R >>')
obj('')
font = obj('<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>')
figure = ['0 0.8 0 rg BT /F1 18 Tf 20 280 Td (Diagram overview) Tj ET']
for i in range(12):
    x = 20 + i * 12
    figure += [f'0.2 0.4 0.8 rg {x} 175 9 14 re f',
               f'0.9 0.3 0.2 rg {x} 55 9 14 re f',
               f'0.6 0.2 0.7 RG 1 w {x+4} 175 m 96 80 l S',
               f'0.2 0.6 0.3 RG {x+4} 69 m 96 175 l S',
               f'0 0 0 rg BT /F1 7 Tf 0 1 -1 0 {x+8} 190 Tm (label{i}) Tj ET',
               f'BT /F1 7 Tf 0 1 -1 0 {x+8} 85 Tm (label{i}) Tj ET']
figure += ['0.9 0 0.9 rg BT /F1 10 Tf 20 25 Td (Figure 1: coloured edges stay visible.) Tj ET',
           '0 0.7 0.7 rg BT /F1 8 Tf 20 6 Td (Ordinary forest text stays selectable.) Tj ET']
prose = 'BT /F1 12 Tf 20 260 Td (The forest remains ordinary readable prose.) Tj ET'
watermark = prose + '\nBT /F1 8 Tf 0 1 -1 0 10 50 Tm (WATERMARK) Tj ET'
pages = []
detached_footer = 'q 1 0 0 0.5 0 140 cm\n' + '\n'.join(figure) + '\nQ\nBT /F1 7 Tf 95 5 Td (4) Tj ET'
overlap = '\n'.join(figure) + '\nBT /F1 8 Tf 35 200 Td (Overlapping ordinary prose must not become a bitmap.) Tj ET'
ambiguous = '\n'.join(figure) + '\nBT /F1 8 Tf 35 260 Td (label0) Tj ET'
for content in ['\n'.join(figure), prose, watermark, detached_footer, overlap, ambiguous]:
    stream = obj(f'<< /Length {len(content)} >>\nstream\n{content}\nendstream')
    pages.append(obj(f'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Resources << /Font << /F1 {font} 0 R >> >> /Contents {stream} 0 R >>'))
objects[1] = f'<< /Type /Pages /Kids [{" ".join(f"{p} 0 R" for p in pages)}] /Count {len(pages)} >>'.encode()
pdf = bytearray(b'%PDF-1.4\n')
offsets = [0]
for i, body in enumerate(objects, 1):
    offsets.append(len(pdf))
    pdf += f'{i} 0 obj\n'.encode() + body + b'\nendobj\n'
xref = len(pdf)
pdf += f'xref\n0 {len(offsets)}\n0000000000 65535 f \n'.encode()
for offset in offsets[1:]:
    pdf += f'{offset:010} 00000 n \n'.encode()
pdf += f'trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()
Path(__file__).with_name('fixtures').joinpath('vector-figure.pdf').write_bytes(pdf)
