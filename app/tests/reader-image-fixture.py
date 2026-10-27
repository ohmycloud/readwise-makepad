#!/usr/bin/env python3
"""Original PNG diagrams for CPU preparation/measurement regressions."""
from pathlib import Path
import struct
import zlib


def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))


def diagram(width, height, exif=b''):
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            pixel = (255, 255, 255, 255)
            if 12 <= x < width-12 and 12 <= y < height-12:
                pixel = (0, 0, 0, 255) if x % 12 == 0 or y % 12 == 0 else (179, 215, 236, 255)
            rows.extend(pixel)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', width, height, 8, 6, 0, 0, 0))
            + (chunk(b'eXIf', exif) if exif else b'')
            + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


path = Path(__file__).with_name('fixtures')
path.mkdir(exist_ok=True)
(path/'reader-diagram.png').write_bytes(diagram(96, 96))
# Little-endian TIFF: orientation = 6 (clockwise 90 degrees).
exif = b'II' + struct.pack('<HIH', 42, 8, 1) + struct.pack('<HHIHHI', 0x112, 3, 1, 6, 0, 0)
(path/'reader-diagram-rotated.png').write_bytes(diagram(96, 64, exif))
