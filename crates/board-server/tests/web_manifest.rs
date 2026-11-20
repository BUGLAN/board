//! 「添加到主屏幕」（PRD §10.1）：入口页链接了应用清单和 apple-touch-icon，清单里的图标文件都在、尺寸对得上，
//! 清单按扩展名得到的类型是 application/manifest+json。
//!
//! 只看 `web/` 里的源文件，不起服务端：发布版本把 `web/dist` 编译进二进制，`static_file` 按扩展名给类型
//! （rust-embed 的 mime-guess，和这里用的是同一个库），清单和图标在 `/api` 之外，不用登录就能取到。

use std::path::Path;

fn web() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web"))
}

/// PNG 头里的宽、高，写成 `宽x高`
fn png_size(src: &str) -> String {
    let bytes = std::fs::read(web().join("public").join(src.trim_start_matches('/'))).unwrap_or_else(|e| panic!("{src}: {e}"));
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{src} 不是 PNG");
    let n = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap());
    format!("{}x{}", n(16), n(20))
}

#[test]
fn index_links_manifest_and_touch_icon() {
    let html = std::fs::read_to_string(web().join("index.html")).unwrap();
    assert!(html.contains(r#"<link rel="manifest" href="/manifest.webmanifest">"#));
    assert!(html.contains(r#"<link rel="apple-touch-icon" href="/icons/apple-touch-icon.png">"#));
    // iOS 的主屏图标：180×180，铺满、不透明（系统自己裁圆角）
    assert_eq!(png_size("/icons/apple-touch-icon.png"), "180x180");
}

#[test]
fn manifest_is_installable() {
    let text = std::fs::read_to_string(web().join("public/manifest.webmanifest")).unwrap();
    let m: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!((m["name"].as_str(), m["display"].as_str(), m["start_url"].as_str()), (Some("Board"), Some("standalone"), Some("/")));
    let icons = m["icons"].as_array().unwrap();
    for (size, purpose) in [("192x192", "any"), ("512x512", "any"), ("512x512", "maskable")] {
        assert!(icons.iter().any(|i| i["sizes"] == size && i["purpose"] == purpose), "清单里缺 {size} {purpose} 的图标");
    }
    // 每张图标都在，尺寸和清单写的一样
    for i in icons {
        assert_eq!(png_size(i["src"].as_str().unwrap()), i["sizes"].as_str().unwrap(), "{}", i["src"]);
    }
    assert_eq!(mime_guess::from_path("manifest.webmanifest").first_or_octet_stream().essence_str(), "application/manifest+json");
}
