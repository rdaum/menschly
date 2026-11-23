import unittest

from PIL import Image

from convert import pack_cell
from extract import Part, render_cell
from levels import room_map


class AssetTests(unittest.TestCase):
    def test_short_pose_keeps_its_rom_baseline(self):
        tiles = Image.new("L", (8, 32), 15)
        tiles.putpixel((0, 23), 3)  # last row of the bottom-left tile
        pose = Part(0, 0, 0, 2, 2, "rows", "-", 0, "")
        frame = render_cell("pose", [pose], tiles, list(range(1, 17)))
        self.assertEqual(frame.getbbox(), (0, 15, 1, 16))

    def test_composite_bounds_include_last_tile_at_a_negative_origin(self):
        tiles = Image.new("L", (8, 24))
        for i in range(3):
            tiles.paste(i + 1, (0, i * 8, 8, i * 8 + 8))
        parts = [Part(-3, -5, 0, 1, 2, "rows", "-", 0, ""),
                 Part(-3, 11, 2, 1, 1, "rows", "-", 0, "")]
        frame = render_cell("tower", parts, tiles, list(range(16)))
        self.assertEqual(frame.getbbox(), (5, 0, 10, 16))
        self.assertEqual([frame.getpixel((7, y)) for y in (0, 8, 15)], [1, 2, 3])

    def test_left_nibble_and_opaque_black(self):
        cell = Image.new("RGBA", (16, 16))
        cell.putpixel((0, 0), (0, 0, 0, 255))
        cell.putpixel((1, 0), (255, 0, 0, 255))
        cell.putpixel((15, 15), (255, 0, 0, 255))
        image, palette = pack_cell(cell, "pose")
        self.assertEqual(image, b"\x12" + bytes(126) + b"\x02")
        self.assertEqual(palette[:9], bytes([0, 0, 0, 0, 0, 0, 255, 0, 0]))

    def test_map_uses_four_wall_quadrants_and_header_openings(self):
        # First interior row solid. Other interior cells use wind code 7.
        grid = room_map(bytes(12) + bytes([255]) * 276, 0x2A)
        self.assertEqual(len(grid), 32 * 26)
        self.assertEqual(grid[:3], bytes([1, 2, 5]))
        self.assertEqual(grid[9:13], bytes([7, 0, 0, 0]))
        self.assertEqual(grid[32:35], bytes([3, 4, 5]))
        self.assertEqual(grid[64:67], bytes([8, 8, 8]))
        self.assertEqual(grid[-2:], bytes([3, 4]))


if __name__ == "__main__":
    unittest.main()
