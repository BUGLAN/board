"""模拟 GitHub OAuth：只用于在本机验证登录流程（launch 配置 github-mock + board-server-auth）。
授权页列出两个账号：选哪个，code 就是哪个账号名；/user 按 code 返回该账号。"""
import html, json, sys, urllib.parse
from http.server import BaseHTTPRequestHandler, HTTPServer

class H(BaseHTTPRequestHandler):
    def _send(self, code, body, ctype="application/json"):
        b = body.encode()
        self.send_response(code)
        self.send_header("content-type", ctype)
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    def do_GET(self):
        u = urllib.parse.urlparse(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        if u.path == "/login/oauth/authorize":
            back = q.get("redirect_uri", "")
            st = q.get("state", "")
            links = "".join(
                f'<p><a id="as-{l}" href="{html.escape(back)}?code={l}&amp;state={html.escape(st)}">以 {l} 授权</a></p>'
                for l in ("buglan", "someone-else")
            )
            self._send(200, f"<!doctype html><meta charset=utf-8><title>模拟 GitHub</title><h1>模拟 GitHub 授权</h1><p>client_id={html.escape(q.get('client_id',''))}</p>{links}", "text/html; charset=utf-8")
        elif u.path == "/user":
            tok = self.headers.get("authorization", "").removeprefix("Bearer gho_")
            self._send(200, json.dumps({"login": tok, "name": f"{tok}（模拟）", "avatar_url": ""}))
        else:
            self._send(404, "{}")

    def do_POST(self):
        n = int(self.headers.get("content-length") or 0)
        form = dict(urllib.parse.parse_qsl(self.rfile.read(n).decode()))
        self._send(200, json.dumps({"access_token": "gho_" + form.get("code", ""), "token_type": "bearer"}))

HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
