#!/usr/bin/env python3
"""
Mock kroak-time API server for testing kroak-time-ticker.

Serves GET /api/state with a 32-singer rotation (plus a few waiting singers).
current_singer only advances when you hit GET /advance — there is no
background timer. This makes tests deterministic: step through singers
one at a time (e.g. via curl) and check the ticker's state after each step,
instead of racing a wall-clock cycle.

Run:  python3 test/mock_server.py
Then: cargo run -- --upstream-url http://localhost:7070/api/state
Step: curl http://localhost:7070/advance   (repeat to move to the next singer)
Reset: curl http://localhost:7070/reset
"""

import json
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer

# (name, artist, title) — title/artist None means "no song queued".
ROTATION_SONGS = [
    ("Dave Grohl",       "Foo Fighters",         "Best of You"),
    ("Stevie Nicks",     "Fleetwood Mac",        "The Chain"),
    ("Freddie Mercury",  "Queen",                "Bohemian Rhapsody"),
    ("Karen O",          "Yeah Yeah Yeahs",      "Maps"),
    ("Tom Waits",        "Tom Waits",            "Downtown Train"),
    ("Alanis Morissette","Alanis Morissette",    "You Oughta Know"),
    ("Jeff Buckley",     "Jeff Buckley",         "Hallelujah"),
    ("Dolly Parton",     "Dolly Parton",         "Jolene"),
    ("Morrissey",        "The Smiths",           "There Is a Light"),
    ("Patti Smith",      "Patti Smith",          "Because the Night"),
    ("Neil Young",       "Neil Young",           "Heart of Gold"),
    ("Liz Phair",        "Liz Phair",            "Supernova"),
    ("David Bowie",      "David Bowie",          "Heroes"),
    ("Courtney Love",    "Hole",                 "Celebrity Skin"),
    ("Lou Reed",         "Velvet Underground",   "Walk on the Wild Side"),
    ("PJ Harvey",        "PJ Harvey",            "Down by the Water"),
    ("Mark Lanegan",     "Screaming Trees",      "Nearly Lost You"),
    ("Cat Power",        "Cat Power",            "The Greatest"),
    ("Nick Cave",        "Nick Cave",            "Into My Arms"),
    ("Björk",            "Björk",                "Human Behaviour"),
    ("Thom Yorke",       "Radiohead",            "Fake Plastic Trees"),
    ("Chrissie Hynde",   "The Pretenders",       "Brass in Pocket"),
    ("Iggy Pop",         "Iggy Pop",             "The Passenger"),
    ("Debbie Harry",     "Blondie",              "Heart of Glass"),
    ("Robert Smith",     "The Cure",             "Just Like Heaven"),
    ("Grace Slick",      "Jefferson Airplane",   "Somebody to Love"),
    ("Joe Strummer",     "The Clash",            "London Calling"),
    ("Siouxsie Sioux",   "Siouxsie and the Banshees", "Kiss Them for Me"),
    ("Ian Curtis",       "Joy Division",         "Love Will Tear Us Apart"),
    ("Kim Deal",         "The Breeders",         "Cannonball"),
    ("Shirley Manson",   "Garbage",              "Only Happy When It Rains"),
    ("Jarvis Cocker",    "Pulp",                 "Common People"),
    ("Bjork's Cousin",   None,                   None),  # no song queued yet
]

WAITING_SONGS = [
    ("Kurt Cobain",  "Nirvana",       "Come as You Are"),
    ("Eddie Vedder", "Pearl Jam",     "Black"),
    ("Chan Marshall", None,           None),
    ("Ella Fitzgerald", "Ella Fitzgerald", "Summertime"),
    ("Sam Cooke",    None,            None),
]

AVG_SONG_SECS = 210  # used only to synthesize a plausible queue_duration_secs
GAP_TIME_SECS = 45

lock = threading.Lock()
current_index = 0
is_playing = True


def singer_obj(entry, is_current):
    name, artist, title = entry
    return {
        "name": name,
        "next_song_artist": artist,
        "next_song_title": title,
        "is_current": is_current,
    }


def build_state():
    with lock:
        idx = current_index
        playing = is_playing

    n = len(ROTATION_SONGS)
    rotation = [singer_obj(entry, i == idx) for i, entry in enumerate(ROTATION_SONGS)]
    current_singer = rotation[idx]
    next_up = rotation[(idx + 1) % n] if n > 1 else None
    waiting = [
        {**singer_obj(entry, False), "is_waiting": True} for entry in WAITING_SONGS
    ]

    # A fixed, plausible-looking "time to get through the queue" number — not a
    # real simulation of remaining time (kept constant so it doesn't decay to
    # a confusing "no time" near the end of a mock cycle).
    queue_duration_secs = n * (AVG_SONG_SECS + GAP_TIME_SECS)

    return {
        "current_singer": current_singer,
        "next_up": next_up,
        "rotation": rotation,
        "waiting": waiting,
        "singer_count": 8,
        "is_playing": playing,
        "status": "ok",
        "queue_duration_secs": queue_duration_secs,
    }


def advance():
    global current_index, is_playing
    with lock:
        current_index = (current_index + 1) % len(ROTATION_SONGS)
        is_playing = True
        return current_index


def reset():
    global current_index, is_playing
    with lock:
        current_index = 0
        is_playing = True


class Handler(BaseHTTPRequestHandler):
    def _json(self, obj):
        payload = json.dumps(obj, ensure_ascii=False).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self):
        if self.path == "/api/state":
            self._json(build_state())
        elif self.path == "/advance":
            idx = advance()
            name = ROTATION_SONGS[idx][0]
            print(f"  -> advanced to singer {idx}: {name}")
            self._json({"current_index": idx, "current_singer": name})
        elif self.path == "/reset":
            reset()
            print("  -> reset to singer 0")
            self._json({"current_index": 0})
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, fmt, *args):
        print(f"  {self.address_string()} {fmt % args}")


if __name__ == "__main__":
    port = 7070
    server = HTTPServer(("0.0.0.0", port), Handler)
    print(f"Mock kroak-time server on http://localhost:{port}/api/state")
    print(f"{len(ROTATION_SONGS)} singers in rotation, {len(WAITING_SONGS)} waiting.")
    print("No auto-advance. Step with: curl http://localhost:7070/advance")
    print("Press Ctrl+C to stop.\n")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
