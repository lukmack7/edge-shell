"""Edge Shell — serwer obecności („body doubling”).

Bardzo mały serwer HTTP bez zależności (sama biblioteka standardowa Pythona).
Każdy Edge Shell co pół minuty wysyła swój stan (fokus / dostępny / z dala)
i pobiera stany pozostałych. Dzięki temu widzisz, że ktoś z Twoich ludzi
właśnie pracuje — i on widzi Ciebie.

Uruchomienie:  PRESENCE_TOKEN=tajne python3 presence.py   (port 7320)
albo Docker:   docker compose up -d   (patrz docker-compose.yml obok)

API (nagłówek Authorization: Bearer <PRESENCE_TOKEN>, jeśli ustawiony):
  POST /status  {"name": "Łukasz", "state": "focus|available|away", "task": "...", "until": "ISO"}
  GET  /status  → [{"name", "state", "task", "until", "seen"}]
Wpisy starsze niż 2 minuty są zwracane jako „offline”, po dobie znikają.
"""

from __future__ import annotations

import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TOKEN = os.environ.get("PRESENCE_TOKEN", "")
PORT = int(os.environ.get("PRESENCE_PORT", "7320"))
STALE_S = 120
FORGET_S = 24 * 3600

_lock = threading.Lock()
_people: dict[str, dict] = {}


class Handler(BaseHTTPRequestHandler):
    server_version = "EdgePresence/1.0"

    def _auth(self) -> bool:
        if not TOKEN:
            return True
        return self.headers.get("Authorization", "") == f"Bearer {TOKEN}"

    def _send(self, code: int, body: object) -> None:
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self) -> None:  # noqa: N802
        if self.path.rstrip("/") == "/health":
            self._send(200, {"status": "ok"})
            return
        if self.path.rstrip("/") != "/status":
            self._send(404, {"error": "not found"})
            return
        if not self._auth():
            self._send(401, {"error": "unauthorized"})
            return
        now = time.time()
        with _lock:
            for name in [n for n, p in _people.items() if now - p["seen"] > FORGET_S]:
                del _people[name]
            out = []
            for p in _people.values():
                state = p["state"] if now - p["seen"] <= STALE_S else "offline"
                out.append({**p, "state": state, "seen": int(now - p["seen"])})
        out.sort(key=lambda p: (p["state"] == "offline", p["name"].lower()))
        self._send(200, out)

    def do_POST(self) -> None:  # noqa: N802
        if self.path.rstrip("/") != "/status":
            self._send(404, {"error": "not found"})
            return
        if not self._auth():
            self._send(401, {"error": "unauthorized"})
            return
        try:
            length = min(int(self.headers.get("Content-Length", "0")), 16 * 1024)
            body = json.loads(self.rfile.read(length) or b"{}")
            name = str(body.get("name", "")).strip()[:40]
            state = str(body.get("state", "available"))
            if not name or state not in ("focus", "available", "away", "offline"):
                raise ValueError
        except (ValueError, json.JSONDecodeError):
            self._send(400, {"error": "expected JSON with name and state"})
            return
        with _lock:
            _people[name] = {
                "name": name,
                "state": state,
                "task": str(body.get("task") or "")[:120],
                "until": str(body.get("until") or "")[:40],
                "seen": time.time(),
            }
        self._send(200, {"ok": True})

    def log_message(self, *_: object) -> None:  # cisza w logach
        pass


if __name__ == "__main__":
    print(f"Edge Shell presence on :{PORT}{' (token required)' if TOKEN else ''}")
    ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
