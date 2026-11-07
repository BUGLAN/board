"""本地预览服务器（禁用缓存）。用法：python3 serve.py [端口]，默认 5178。

直接双击 index.html 也能运行；用服务器的好处是改完代码刷新即生效，
并且可以在多个窗口打开同一地址，观察白板的实时同步。
"""
import http.server
import os
import sys


class NoCacheHandler(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


if __name__ == "__main__":
    os.chdir(os.path.dirname(os.path.abspath(__file__)))
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 5178
    print(f"Board 原型：http://localhost:{port}")
    http.server.ThreadingHTTPServer(("", port), NoCacheHandler).serve_forever()
