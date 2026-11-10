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

Wysyłanie plików z Półki na telefon (koncepcja 6):
  POST /share   (bearer wymagany) — body: plik, nagłówek X-Filename (zakodowany URL-encode)
                → {"id": "..."}  · limit 50 MB, żyje 10 minut, tylko w pamięci
  GET  /share/{id} — BEZ tokenu (telefon ma dostać sam link), zwraca plik do pobrania
"""

from __future__ import annotations

import json
import mimetypes
import os
import secrets
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TOKEN = os.environ.get("PRESENCE_TOKEN", "")
PORT = int(os.environ.get("PRESENCE_PORT", "7320"))
STALE_S = 120
FORGET_S = 24 * 3600
SHARE_TTL_S = 10 * 60
SHARE_MAX = 50 * 1024 * 1024

_lock = threading.Lock()
_people: dict[str, dict] = {}
# id → (dane, deadline, nazwa pliku); tylko pamięć — po restarcie serwera linki gasną.
_shares: dict[str, tuple[bytes, float, str]] = {}


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
        # Pobranie udostępnionego pliku — celowo bez tokenu: link idzie na telefon.
        if self.path.startswith("/share/"):
            self._share_get()
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
        if self.path.rstrip("/") == "/share":
            self._share_post()
            return
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

    # --- Półka → telefon: upload z tokenem, pobranie bez ------------------

    def _share_post(self) -> None:
        if TOKEN and not self._auth():
            self._send(401, {"error": "unauthorized"})
            return
        name = urllib.parse.unquote(self.headers.get("X-Filename", "")).strip()[:120] or "plik"
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            length = 0
        if length <= 0 or length > SHARE_MAX:
            self._send(413, {"error": f"plik musi mieć 1 B–{SHARE_MAX // (1024 * 1024)} MB"})
            return
        data = self.rfile.read(length)
        mime = mimetypes.guess_type(name)[0] or "application/octet-stream"
        sid = secrets.token_urlsafe(9)
        with _lock:
            # Przy okazji sprzątamy przedawnione udostępnienia.
            now = time.time()
            for k in [k for k, (_, dl, _) in _shares.items() if dl <= now]:
                del _shares[k]
            _shares[sid] = (data, now + SHARE_TTL_S, name)
        self._send(200, {"id": sid})

    def _share_get(self) -> None:
        sid = self.path[len("/share/"):].split("?")[0].split("/")[0]
        now = time.time()
        with _lock:
            item = _shares.get(sid)
            if item and item[1] <= now:
                del _shares[sid]
                item = None
        if not item:
            self._send(404, {"error": "link wygasł (ważny 10 minut)"})
            return
        data, _, name = item
        mime = mimetypes.guess_type(name)[0] or "application/octet-stream"
        quoted = urllib.parse.quote(name)
        self.send_response(200)
        self.send_header("Content-Type", mime)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Content-Disposition", f"attachment; filename*=UTF-8''{quoted}")
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *_: object) -> None:  # cisza w logach
        pass


if __name__ == "__main__":
    print(f"Edge Shell presence on :{PORT}{' (token required)' if TOKEN else ''}")
    ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
