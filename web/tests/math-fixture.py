"""Owned, minimal text-layer PDF with real fraction/script geometry, no TeX source.
The source order intentionally differs from visual order. No fonts/network needed.
"""
import pathlib
import sys

content = r"""BT /F1 12 Tf 40 250 Td (A forest and solitude before the equation.) Tj ET
BT /F2 12 Tf 60 200 Td (f\(x\)) Tj ET
BT /F1 12 Tf 90 200 Td (=) Tj ET
BT /F2 12 Tf 112 207 Td (a) Tj ET
BT /F1 8 Tf 120 212 Td (2) Tj ET
BT /F2 12 Tf 114 192 Td (b) Tj ET
0.5 w 109 203 m 133 203 l S
BT /F1 12 Tf 145 200 Td (+) Tj ET
BT /F2 12 Tf 160 200 Td (y) Tj ET
BT /F2 8 Tf 167 196 Td (k) Tj ET
BT /F1 12 Tf 250 200 Td (\(1\)) Tj ET
BT /F1 12 Tf 40 145 Td (Dimensions: ) Tj ET
BT /F2 12 Tf 110 145 Td (W) Tj ET
BT /F2 8 Tf 121 141 Td (i) Tj ET
BT /F2 8 Tf 121 148.5 Td (Q) Tj ET
BT /F1 12 Tf 133 145 Td (=) Tj ET
BT /F2 12 Tf 146 145 Td (R) Tj ET
BT /F2 8 Tf 155 149 Td (d+k) Tj ET
BT /F1 12 Tf 180 145 Td ( used here.) Tj ET
BT /F1 12 Tf 40 100 Td (A murmur in the forest after the equation.) Tj ET
""".encode("ascii")
objects = [
    b"<< /Type /Catalog /Pages 2 0 R >>",
    b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 340 300] /Resources << /Font << /F1 4 0 R /F2 5 0 R >> >> /Contents 6 0 R >>",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /Times-Roman /Encoding /WinAnsiEncoding >>",
    b"<< /Type /Font /Subtype /Type1 /BaseFont /CMMI10 /Encoding /WinAnsiEncoding >>",
    b"<< /Length " + str(len(content)).encode() + b" >>\nstream\n" + content + b"endstream",
]
out = bytearray(b"%PDF-1.4\n")
offsets = [0]
for i, obj in enumerate(objects, 1):
    offsets.append(len(out))
    out.extend(f"{i} 0 obj\n".encode() + obj + b"\nendobj\n")
xref = len(out)
out.extend(f"xref\n0 {len(offsets)}\n0000000000 65535 f \n".encode())
for offset in offsets[1:]:
    out.extend(f"{offset:010d} 00000 n \n".encode())
out.extend(f"trailer\n<< /Root 1 0 R /Size {len(offsets)} >>\nstartxref\n{xref}\n%%EOF\n".encode())
pathlib.Path(sys.argv[1]).write_bytes(out)
