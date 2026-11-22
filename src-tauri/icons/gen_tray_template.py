#!/usr/bin/env python3
"""Render the committed Cut Glass black-on-transparent menu-bar silhouette.

macOS tints template images for the menu-bar appearance. Authoring only:
requires macOS sips; no runtime/build dependency.
"""
from pathlib import Path
import subprocess

folder = Path(__file__).resolve().parent
subprocess.run(["sips", "-s", "format", "png", str(folder / "tray-template.svg"),
                "--out", str(folder / "tray-template.png")], check=True)
