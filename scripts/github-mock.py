"""模拟 GitHub OAuth：只用于在本机验证登录流程（launch 配置 github-mock + board-server-auth）。
授权页列出两个账号：选哪个，code 就是哪个账号名；/user 按 code 返回该账号。
另有一个同名换 id 的入口（buglan@2），用来验证改名前后的旧用户名被抢注时不能登录。"""
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
            accounts = (("buglan", "buglan", "以 buglan 授权"), ("someone-else", "someone-else", "以 someone-else 授权"), ("buglan-id2", "buglan@2", "以同名 buglan、id=2 授权（模拟旧用户名被抢注）"))
            links = "".join(
                f'<p><a id="as-{anchor}" href="{html.escape(back)}?code={html.escape(code)}&amp;state={html.escape(st)}">{label}</a></p>'
                for anchor, code, label in accounts
            )
            self._send(200, f"<!doctype html><meta charset=utf-8><title>模拟 GitHub</title><h1>模拟 GitHub 授权</h1><p>client_id={html.escape(q.get('client_id',''))}</p>{links}", "text/html; charset=utf-8")
        elif u.path == "/user":
            who = self.headers.get("authorization", "").removeprefix("Bearer gho_")
            login, _, raw_id = who.partition("@")
            ids = {"buglan": 1, "someone-else": 2}
            account_id = int(raw_id) if raw_id.isdigit() else ids.get(login, 0)
            self._send(200, json.dumps({"id": account_id, "login": login, "name": f"{login}（模拟）", "avatar_url": ""}))
        else:
            self._send(404, "{}")

    def do_POST(self):
        n = int(self.headers.get("content-length") or 0)
        form = dict(urllib.parse.parse_qsl(self.rfile.read(n).decode()))
        self._send(200, json.dumps({"access_token": "gho_" + form.get("code", ""), "token_type": "bearer"}))

HTTPServer(("127.0.0.1", int(sys.argv[1])), H).serve_forever()
