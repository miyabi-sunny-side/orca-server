"""Check release metadata updates without publishing or replacing the changelog."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'packaging'))
from release_notes import update_image_reference


class ReleaseNotes(unittest.TestCase):
    def test_updates_only_the_leading_image_reference(self):
        body = 'Image: `ghcr.io/example/app:0.1.0@sha256:old` · [Source](https://example.org/source)\n\n**Changes**\n- Keep `sha256:old` here.\n'
        expected = 'Image: `ghcr.io/example/app:0.1.0@sha256:new` · [Source](https://example.org/source)\n\n**Changes**\n- Keep `sha256:old` here.\n'
        self.assertEqual(update_image_reference(body, 'ghcr.io/example/app:0.1.0@sha256:new'), expected)
        self.assertEqual(update_image_reference(expected, 'ghcr.io/example/app:0.1.0@sha256:new'), expected)

    def test_unrecognized_notes_are_not_overwritten(self):
        for body in ['', 'Custom notes\nImage: `manual`', 'Image: missing delimiter']:
            with self.subTest(body=body), self.assertRaises(ValueError):
                update_image_reference(body, 'ghcr.io/example/app:0.1.0@sha256:new')


if __name__ == '__main__':
    unittest.main()
