#!/usr/bin/env python3
"""Asset regressions. Run: python3 -m unittest discover -s scripts -p test_app_icons.py"""
import json
from pathlib import Path
import shutil
import struct
import subprocess
import unittest
import xml.etree.ElementTree as ET

from generate_app_icons import BG, DENSITIES, NS, RES, ROOT, SIZES


def png_header(data):
    if data[:8] != b'\x89PNG\r\n\x1a\n':
        raise AssertionError('Not a PNG')
    width, height, depth, colour = struct.unpack('>IIBB', data[16:26])
    return width, height, depth, colour


def alpha(path):
    return subprocess.run(['magick', str(path), '-alpha', 'extract', '-depth', '8', 'gray:-'],
                          check=True, stdout=subprocess.PIPE).stdout


class AppIconTests(unittest.TestCase):
    def test_masters_are_three_colour_filled_vector_art(self):
        for name in ('icon.svg', 'icon-small.svg'):
            with self.subTest(name=name):
                svg = ET.parse(ROOT / RES / name).getroot()
                self.assertEqual(svg.get('viewBox'), '0 0 256 256')
                self.assertEqual(svg.find(f'{NS}rect').get('fill'), BG)
                self.assertIsNone(svg.find(f'{NS}rect').get('rx'))
                colours = set()
                for element in svg.iter():
                    self.assertNotIn(element.tag.removeprefix(NS), ('text', 'image', 'filter', 'mask', 'use'))
                    self.assertNotIn('stroke', element.attrib)
                    if 'fill' in element.attrib:
                        colours.add(element.get('fill'))
                self.assertEqual(colours, {'#F8DDD2', '#DF8267', '#57332D'})

    def test_desktop_sizes_and_workspace_mirror(self):
        for size in SIZES:
            with self.subTest(size=size):
                image = (ROOT / RES / f'icon_{size}.png').read_bytes()
                self.assertEqual(png_header(image), (size, size, 8, 6))
                self.assertEqual(image, (ROOT / 'resources' / f'icon_{size}.png').read_bytes())

    @unittest.skipUnless(shutil.which('magick'), 'ImageMagick required for pixel tests')
    def test_desktop_has_transparent_corners_and_opaque_centre(self):
        for size in (16, 32, 1024):
            with self.subTest(size=size):
                pixels = alpha(ROOT / RES / f'icon_{size}.png')
                self.assertEqual(len(pixels), size * size)
                self.assertEqual(pixels[0], 0)
                self.assertEqual(pixels[(size // 2) * size + size // 2], 255)

    def test_windows_ico_contains_all_sizes_and_matches_pngs(self):
        data = (ROOT / RES / 'icon.ico').read_bytes()
        reserved, kind, count = struct.unpack_from('<HHH', data)
        self.assertEqual((reserved, kind, count), (0, 1, 7))
        actual_sizes = []
        for i in range(count):
            w, h, _, _, planes, depth, length, offset = struct.unpack_from('<BBBBHHII', data, 6 + i * 16)
            size = w or 256
            actual_sizes.append(size)
            self.assertEqual(h or 256, size)
            self.assertEqual((planes, depth), (1, 32))
            png = data[offset:offset + length]
            self.assertEqual(png_header(png)[:2], (size, size))
            if size in SIZES:
                self.assertEqual(png, (ROOT / RES / f'icon_{size}.png').read_bytes())
        self.assertEqual(actual_sizes, [16, 24, 32, 48, 64, 128, 256])

    def test_macos_icns_has_native_and_retina_sizes(self):
        data = (ROOT / RES / 'icon.icns').read_bytes()
        self.assertEqual(data[:4], b'icns')
        self.assertEqual(struct.unpack_from('>I', data, 4)[0], len(data))
        sizes, offset = {}, 8
        while offset < len(data):
            name = data[offset:offset + 4].decode()
            length = struct.unpack_from('>I', data, offset + 4)[0]
            self.assertGreater(length, 8)
            png = data[offset + 8:offset + length]
            size = png_header(png)[0]
            self.assertEqual(png, (ROOT / RES / f'icon_{size}.png').read_bytes())
            sizes[name] = size
            offset += length
        self.assertEqual(offset, len(data))
        self.assertEqual(sizes, {'icp4': 16, 'icp5': 32, 'icp6': 64, 'ic07': 128,
                                'ic08': 256, 'ic09': 512, 'ic10': 1024,
                                'ic11': 32, 'ic12': 64, 'ic13': 256, 'ic14': 512})

    def test_ios_catalog_dimensions_and_no_alpha_channel(self):
        folder = ROOT / 'app/packaging/ios/icons/Assets.xcassets/AppIcon.appiconset'
        catalog = json.loads((folder / 'Contents.json').read_text())
        self.assertEqual(len(catalog['images']), 18)
        for image in catalog['images']:
            with self.subTest(image=image):
                pixels = int(float(image['size'].split('x')[0]) * int(image['scale'][0]))
                self.assertEqual(png_header((folder / image['filename']).read_bytes()),
                                 (pixels, pixels, 8, 2))
        # cargo-makepad additionally copies these exact names as loose iOS icons.
        for size in (120, 180, 152, 167):
            self.assertTrue((folder / f'AppIcon{size}x{size}.png').is_file())

    def test_android_adaptive_resources_are_connected(self):
        folder = ROOT / RES / 'android/res'
        xml = ET.parse(folder / 'mipmap-anydpi-v26/ic_launcher.xml').getroot()
        attribute = '{http://schemas.android.com/apk/res/android}drawable'
        self.assertEqual(xml.find('foreground').get(attribute), '@mipmap/ic_launcher_foreground')
        self.assertEqual(xml.find('background').get(attribute), '@color/ic_launcher_background')
        self.assertEqual(ET.parse(folder / 'values/icon_colors.xml').getroot().find('color').text, BG)
        for density, scale in DENSITIES.items():
            for name, base in [('ic_launcher.png', 48), ('ic_launcher_foreground.png', 108)]:
                size = int(base * scale)
                self.assertEqual(png_header((folder / f'mipmap-{density}' / name).read_bytes())[:2], (size, size))

    @unittest.skipUnless(shutil.which('magick'), 'ImageMagick required for pixel tests')
    def test_android_foreground_stays_inside_guaranteed_safe_circle(self):
        for density, scale in DENSITIES.items():
            with self.subTest(density=density):
                path = ROOT / RES / 'android/res' / f'mipmap-{density}/ic_launcher_foreground.png'
                size = int(108 * scale)
                pixels = alpha(path)
                radius = 33 * scale + 1  # One pixel tolerance for antialiasing.
                occupied = [(i % size + .5 - size / 2, i // size + .5 - size / 2)
                            for i, a in enumerate(pixels) if a]
                self.assertTrue(occupied)
                self.assertLessEqual(max(x*x + y*y for x, y in occupied), radius * radius)


if __name__ == '__main__':
    unittest.main()
