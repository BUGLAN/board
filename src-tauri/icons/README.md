# 图标

`source.svg` 是源文件（1024×1024）。重新生成全部图标：

```sh
python scripts/build-icons.py   # 需要 Python 3 和 Pillow，先 npm ci
```

Windows 的 `icon.ico` 里 16/20/24/32/40/48 六帧按像素网格单独绘制，不要直接用
`npx tauri icon` 覆盖，否则小尺寸会重新变糊。
