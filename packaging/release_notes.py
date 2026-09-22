"""Refresh the published image reference while retaining existing release notes."""
import re
import sys
from pathlib import Path


def update_image_reference(body: str, image: str) -> str:
    updated, count = re.subn(r'^Image: `[^`]+`', lambda _: f'Image: `{image}`', body, count=1)
    if count != 1:
        raise ValueError('Release notes must start with the generated Image reference')
    return updated


if __name__ == '__main__':
    path = Path(sys.argv[1])
    path.write_text(update_image_reference(path.read_text(), sys.argv[2]))
