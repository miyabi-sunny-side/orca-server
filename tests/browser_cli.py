"""Browser + official Orca: python3 tests/browser_cli.py APPDIR OUTPUT_DIR [BINARY]."""
import os
from pathlib import Path
import sys
from browser_queue import run
from printer_start import REPO

if __name__=='__main__':
    binary=sys.argv[3] if len(sys.argv)>3 else Path(os.environ.get('CARGO_TARGET_DIR',REPO/'target'))/'debug/orca-server'
    run(binary,sys.argv[2],appdir=Path(sys.argv[1]).resolve())
