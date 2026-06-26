#!/usr/bin/env python3
"""
Mock kroak-time API server for testing kroak-time-ticker.

Serves GET /api/state with a 20-singer rotation.
Run:  python3 test/mock_server.py
Then: cargo run -- --upstream-url http://localhost:7070/api/state
"""

import json
from http.server import BaseHTTPRequestHandler, HTTPServer

STATE = {
    "current_singer": {
        "name": "Dave Grohl",
        "next_song_artist": "Foo Fighters",
        "next_song_title": "Best of You",
        "is_current": True
    },
    "next_up": {
        "name": "Stevie Nicks",
        "next_song_artist": "Fleetwood Mac",
        "next_song_title": "The Chain",
        "is_current": False
    },
    "rotation": [
        {"name": "Dave Grohl",      "next_song_artist": "Foo Fighters",      "next_song_title": "Best of You",              "is_current": True},
        {"name": "Stevie Nicks",    "next_song_artist": "Fleetwood Mac",      "next_song_title": "The Chain",               "is_current": False},
        {"name": "Freddie Mercury", "next_song_artist": "Queen",              "next_song_title": "Bohemian Rhapsody",        "is_current": False},
        {"name": "Karen O",         "next_song_artist": "Yeah Yeah Yeahs",    "next_song_title": "Maps",                    "is_current": False},
        {"name": "Tom Waits",       "next_song_artist": "Tom Waits",          "next_song_title": "Downtown Train",          "is_current": False},
        {"name": "Alanis Morissette","next_song_artist": "Alanis Morissette", "next_song_title": "You Oughta Know",         "is_current": False},
        {"name": "Jeff Buckley",    "next_song_artist": "Jeff Buckley",       "next_song_title": "Hallelujah",              "is_current": False},
        {"name": "Dolly Parton",    "next_song_artist": "Dolly Parton",       "next_song_title": "Jolene",                  "is_current": False},
        {"name": "Morrissey",       "next_song_artist": "The Smiths",         "next_song_title": "There Is a Light",        "is_current": False},
        {"name": "Patti Smith",     "next_song_artist": "Patti Smith",        "next_song_title": "Because the Night",       "is_current": False},
        {"name": "Neil Young",      "next_song_artist": "Neil Young",         "next_song_title": "Heart of Gold",           "is_current": False},
        {"name": "Liz Phair",       "next_song_artist": "Liz Phair",          "next_song_title": "Supernova",               "is_current": False},
        {"name": "David Bowie",     "next_song_artist": "David Bowie",        "next_song_title": "Heroes",                  "is_current": False},
        {"name": "Courtney Love",   "next_song_artist": "Hole",               "next_song_title": "Celebrity Skin",          "is_current": False},
        {"name": "Lou Reed",        "next_song_artist": "Velvet Underground",  "next_song_title": "Walk on the Wild Side",  "is_current": False},
        {"name": "PJ Harvey",       "next_song_artist": "PJ Harvey",          "next_song_title": "Down by the Water",       "is_current": False},
        {"name": "Mark Lanegan",    "next_song_artist": "Screaming Trees",    "next_song_title": "Nearly Lost You",         "is_current": False},
        {"name": "Cat Power",       "next_song_artist": "Cat Power",          "next_song_title": "The Greatest",            "is_current": False},
        {"name": "Nick Cave",       "next_song_artist": "Nick Cave",          "next_song_title": "Into My Arms",            "is_current": False},
        {"name": "Björk",           "next_song_artist": "Björk",              "next_song_title": "Human Behaviour",         "is_current": False},
    ],
    "singer_count": 8,
    "is_playing": True,
    "status": "ok"
}

PAYLOAD = json.dumps(STATE, ensure_ascii=False).encode("utf-8")


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path == "/api/state":
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(PAYLOAD)))
            self.send_header("Access-Control-Allow-Origin", "*")
            self.end_headers()
            self.wfile.write(PAYLOAD)
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, fmt, *args):
        print(f"  {self.address_string()} {fmt % args}")


if __name__ == "__main__":
    port = 7070
    server = HTTPServer(("0.0.0.0", port), Handler)
    print(f"Mock kroak-time server on http://localhost:{port}/api/state")
    print("Press Ctrl+C to stop.\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
