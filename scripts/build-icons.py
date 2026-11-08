#!/usr/bin/env python3
"""从 src-tauri/icons/source.svg 重新生成桌面端图标。

Windows 任务栏图标来自 exe 内嵌的 icon.ico。任务栏按 24 DIP 绘制窗口图标
（100/125/150/175/200% 分别对应 24/30/36/42/48px），桌面端会在窗口创建后
按当前缩放从 exe 图标资源加载对应尺寸帧。把 1024 的 SVG 直接缩小到这些尺寸时，
4 根细竖条会糊成一团，所以小尺寸按目标像素网格单独绘制；64 及以上仍然用矢量渲染。

用法（仓库根目录，先 npm ci；需要 Python 3 和 Pillow）：

    python scripts/build-icons.py
"""
from __future__ import annotations

import io
import shutil
import struct
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"
WORK = ROOT / "target" / "icon-build"

# source.svg 的几何比例（viewBox 1024）：4 根竖条从 x=232 开始，每根宽 112、间隔 56；
# 底部在 y=768，高度分别为 512/358/236/420；外框圆角 200。
COLORS = ["#8F9AB4", "#FF9A3C", "#4DB5FF", "#3DDC97"]
HEIGHTS = [512 / 1024, 358 / 1024, 236 / 1024, 420 / 1024]
BAR_WIDTH = 112 / 1024
BAR_GAP = 56 / 1024
MARGIN = 64 / 1024
RADIUS = 200 / 1024

# 16/20/24/32/40/48 是 Windows 常规图标尺寸；30/36/42 是任务栏在 125/150/175% 缩放下的尺寸。
SMALL_SIZES = (16, 20, 24, 30, 32, 36, 40, 42, 48)
BG_TOP = (0x26, 0x27, 0x2C)
BG_BOTTOM = (0x0C, 0x0D, 0x0F)
SS = 8  # 超采样倍数


def render_small(size: int) -> Image.Image:
    """按整数像素坐标绘制小尺寸图标（竖条和圆角都对齐像素网格）。"""
    bar_w = max(1, round(BAR_WIDTH * size))
    gap = max(1, round(BAR_GAP * size))
    margin = max(1, round(MARGIN * size))
    radius = max(1, round(RADIUS * (size - 2 * margin)))

    canvas = Image.new("RGBA", (size * SS, size * SS), (0, 0, 0, 0))
    gradient = Image.new("RGBA", (size * SS, size * SS))
    grad_draw = ImageDraw.Draw(gradient)
    for y in range(size * SS):
        t = y / (size * SS - 1)
        color = tuple(round(BG_TOP[i] + (BG_BOTTOM[i] - BG_TOP[i]) * t) for i in range(3)) + (255,)
        grad_draw.line([(0, y), (size * SS, y)], fill=color)

    mask = Image.new("L", (size * SS, size * SS), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        [margin * SS, margin * SS, (size - margin) * SS, (size - margin) * SS],
        radius=radius * SS,
        fill=255,
    )
    canvas = Image.composite(gradient, canvas, mask)

    draw = ImageDraw.Draw(canvas)
    content_w = 4 * bar_w + 3 * gap
    x = round((size - content_w) / 2)
    bottom = round(0.75 * size)
    for i in range(4):
        top = bottom - round(HEIGHTS[i] * size)
        draw.rounded_rectangle(
            [x * SS, top * SS, (x + bar_w) * SS, bottom * SS],
            radius=(bar_w / 2) * SS,
            fill=COLORS[i],
        )
        x += bar_w + gap

    return canvas.resize((size, size), Image.Resampling.BOX)


def render_large() -> dict[int, Path]:
    """用 Tauri CLI 从 source.svg 生成 64 及以上的 PNG（resvg 矢量渲染）。"""
    out = WORK / "large"
    if out.exists():
        shutil.rmtree(out)
    npx = shutil.which("npx")
    if not npx:
        sys.exit("找不到 npx：先在仓库根目录运行 npm ci")
    cmd = [npx, "--no-install", "tauri", "icon", str(ICONS / "source.svg"), "-o", str(out), "-p", "64,128,256,512"]
    subprocess.run(cmd, cwd=ROOT, check=True)
    return {size: out / f"{size}x{size}.png" for size in (64, 128, 256, 512)}


def write_ico(frames: list[tuple[int, Path]], dest: Path) -> None:
    """把多张 PNG 按尺寸写进 ICO（Vista 起支持 PNG 帧，原图标也是这么做的）。"""
    images: list[tuple[int, bytes]] = []
    for size, path in frames:
        im = Image.open(path).convert("RGBA")
        if im.size != (size, size):
            sys.exit(f"{path} 尺寸不是 {size}x{size}")
        buf = io.BytesIO()
        im.save(buf, format="PNG", optimize=True)
        images.append((size, buf.getvalue()))

    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries = b""
    data = b""
    for size, png in images:
        width = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", width, width, 0, 0, 1, 32, len(png), offset)
        data += png
        offset += len(png)
    dest.write_bytes(header + entries + data)


def main() -> None:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    small_dir = WORK / "small"
    small_dir.mkdir(parents=True, exist_ok=True)
    for size in SMALL_SIZES:
        render_small(size).save(small_dir / f"{size}x{size}.png")

    large = render_large()

    write_ico([(size, small_dir / f"{size}x{size}.png") for size in SMALL_SIZES]
              + [(size, large[size]) for size in (64, 128, 256)],
              ICONS / "icon.ico")

    shutil.copyfile(small_dir / "32x32.png", ICONS / "32x32.png")
    shutil.copyfile(large[128], ICONS / "128x128.png")
    shutil.copyfile(large[256], ICONS / "128x128@2x.png")
    shutil.copyfile(large[512], ICONS / "icon.png")
    print("✓ 图标已生成：icon.ico（16/20/24/30/32/36/40/42/48/64/128/256）+ 32/128/256/512 PNG")
    print("  icon.icns 未改动（macOS 图标）")


if __name__ == "__main__":
    main()
