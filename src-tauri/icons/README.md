# 图标

`source.svg` 是源文件（1024×1024）。重新生成全部图标：

```sh
python scripts/build-icons.py   # 需要 Python 3 和 Pillow，先 npm ci
```

Windows 的 `icon.ico` 里 16/20/24/30/32/36/40/42/48 各帧按像素网格单独绘制，不要直接用
`npx tauri icon` 覆盖，否则小尺寸会重新变糊。其中 24/30/36/42/48 是任务栏在
100/125/150/175/200% 缩放下的显示尺寸，`src-tauri/src/lib.rs` 的 `apply_taskbar_icon()`
会在窗口创建后按当前缩放加载对应帧。
