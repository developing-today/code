#!/usr/bin/env python3
"""Play checkers and chess between two id processes over the id network.

Slice 2 of #15111 (third slice of #14862). For each topology and each
game this script starts a world host with the game's module, invites
two players, joins each from its own process and working directory,
and plays a game to completion with keystrokes sent as `/input <hex>`.

Topologies:
  host-join  The host player's join runs from the host's own data
             directory, next to `id serve`: one id home hosts and plays,
             the other only joins.
  central    A separate data directory runs `id serve`; both players
             join it from their own directories.

Every move is checked on both sides: after the mover's input, both
join sessions must read the same records (seats, turn, board, plies),
and the board must match this script's own model of the game. Before
the game, out-of-turn input from the second player and input from a
watcher are sent and the records must not change. After the game, a
restart, a disconnect (the second player quits) and a move against the
AI that takes the open seat check that a leave is surfaced.

Exit status 0 only when every game in every topology finishes; the
last line is `RESULT verdict=PASS ...` or `RESULT verdict=FAIL ...`.

Usage:
  board-games-two-player.py --id-bin PATH --checkers-wasm PATH
      --chess-wasm PATH [--dir DIR] [--port-base N] [--timeout S]
      [--topology host-join|central|both] [--game checkers|chess|both]
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import re
import secrets
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

NODE_RE = re.compile(r"^[0-9a-f]{64}$")


class Failure(Exception):
    """A check failed; the message says which."""


def log(message: str) -> None:
    print(message, flush=True)


# --- the id processes --------------------------------------------------------


class Host:
    """One `id serve --world` process with a game module."""

    def __init__(
        self,
        id_bin: str,
        data_dir: Path,
        world: str,
        module: Path,
        port: int,
        timeout: float,
    ):
        self.id_bin = id_bin
        self.data_dir = data_dir
        self.world = world
        self.port = port
        self.timeout = timeout
        self.token = secrets.token_hex(16)
        data_dir.mkdir(parents=True, exist_ok=True)
        self.log_path = data_dir.parent / f"{data_dir.name}-serve.log"
        env = dict(os.environ, ID_WORLD_ADMIN_TOKEN=self.token)
        self.proc = subprocess.Popen(
            [
                id_bin,
                "--data-dir",
                str(data_dir),
                "serve",
                "--world",
                "--world-name",
                world,
                "--world-module",
                str(module.resolve()),
                "--world-cap",
                "players",
                "--iroh-port",
                str(port),
                "--no-relay",
                "--no-gossip",
                "--no-mdns",
            ],
            stdout=open(self.log_path, "wb"),  # noqa: SIM115 - lives as long as serve
            stderr=subprocess.STDOUT,
            env=env,
        )
        self.node = self._wait_node()
        self._wait_ready()

    def addr_args(self) -> list[str]:
        return ["--world", self.world, "--addr", f"127.0.0.1:{self.port}", "--no-relay"]

    def _alive(self, what: str) -> None:
        if self.proc.poll() is not None:
            raise Failure(f"serve exited {what}; see {self.log_path}")

    def _wait_node(self) -> str:
        deadline = time.time() + self.timeout
        while time.time() < deadline:
            self._alive("during startup")
            out = subprocess.run(
                [self.id_bin, "--data-dir", str(self.data_dir), "id"],
                capture_output=True,
                text=True,
                check=False,
            ).stdout.strip()
            if NODE_RE.match(out):
                return out
            time.sleep(1)
        raise Failure(
            f"serve printed no node id within {self.timeout}s; see {self.log_path}"
        )

    def _wait_ready(self) -> None:
        deadline = time.time() + self.timeout
        while time.time() < deadline:
            self._alive("before it was reachable")
            done = subprocess.run(
                [
                    self.id_bin,
                    "world",
                    "list",
                    self.node,
                    "--addr",
                    f"127.0.0.1:{self.port}",
                    "--no-relay",
                ],
                capture_output=True,
                check=False,
            )
            if done.returncode == 0:
                return
            time.sleep(1)
        raise Failure(
            f"serve not reachable within {self.timeout}s; see {self.log_path}"
        )

    def invite(self, name: str) -> str:
        env = dict(os.environ, ID_WORLD_ADMIN_TOKEN=self.token)
        done = subprocess.run(
            [
                self.id_bin,
                "world",
                "invite",
                self.node,
                "--name",
                name,
                "--role",
                "player",
                *self.addr_args(),
            ],
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )
        capability = done.stdout.strip()
        if done.returncode != 0 or not capability:
            raise Failure(f"invite {name} failed: {done.stderr.strip()[-400:]}")
        return capability

    def stop(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()


class Player:
    """One `id world join --command-mode --json` process."""

    def __init__(self, host: Host, name: str, home: Path, timeout: float):
        self.name = name
        self.timeout = timeout
        home.mkdir(parents=True, exist_ok=True)
        capability = host.invite(name)
        env = dict(os.environ, ID_WORLD_CAPABILITY=capability)
        env.pop("ID_WORLD_ADMIN_TOKEN", None)
        self.transcript = open(home / f"join-{name}.jsonl", "w", encoding="utf-8")  # noqa: SIM115
        self.err_path = home / f"join-{name}.err"
        self.proc = subprocess.Popen(
            [
                host.id_bin,
                "world",
                "join",
                host.node,
                *host.addr_args(),
                "--command-mode",
                "--json",
            ],
            cwd=home,
            env=env,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=open(self.err_path, "wb"),  # noqa: SIM115 - lives as long as join
            text=True,
            bufsize=1,
        )
        self.frames: queue.Queue = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()
        self._first_frame()

    def _read(self) -> None:
        assert self.proc.stdout is not None
        for line in self.proc.stdout:
            self.transcript.write(line)
            self.transcript.flush()
            try:
                self.frames.put(json.loads(line))
            except json.JSONDecodeError:
                continue
        self.frames.put(None)

    def _first_frame(self) -> None:
        try:
            frame = self.frames.get(timeout=self.timeout)
        except queue.Empty:
            raise Failure(
                f"{self.name}: no server frame within {self.timeout}s; see {self.err_path}"
            ) from None
        if frame is None:
            raise Failure(
                f"{self.name}: join ended before its first frame; see {self.err_path}"
            )

    def send(self, line: str) -> None:
        assert self.proc.stdin is not None
        if self.proc.poll() is not None:
            raise Failure(f"{self.name}: join exited; see {self.err_path}")
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def keys(self, text: str) -> None:
        self.send("/input " + text.encode().hex())

    def records(self) -> dict:
        """Ask the host for the world's records and return them."""
        while True:  # drop frames that arrived since the last read
            try:
                if self.frames.get_nowait() is None:
                    raise Failure(
                        f"{self.name}: join stream ended; see {self.err_path}"
                    )
            except queue.Empty:
                break
        self.send("/records")
        deadline = time.time() + self.timeout
        while time.time() < deadline:
            try:
                frame = self.frames.get(timeout=max(0.1, deadline - time.time()))
            except queue.Empty:
                break
            if frame is None:
                raise Failure(f"{self.name}: join stream ended; see {self.err_path}")
            if isinstance(frame, dict) and frame.get("type") == "records":
                return frame.get("records") or {}
            if isinstance(frame, dict) and frame.get("type") == "error":
                raise Failure(f"{self.name}: records refused: {frame}")
        raise Failure(f"{self.name}: no records reply within {self.timeout}s")

    def wait_for(self, what: str, check) -> dict:
        deadline = time.time() + self.timeout
        last: dict = {}
        while time.time() < deadline:
            last = self.records()
            if check(last):
                return last
            time.sleep(0.2)
        raise Failure(
            f"{self.name}: timed out waiting for {what}; last records {json.dumps(last, sort_keys=True)}"
        )

    def quit(self) -> None:
        if self.proc.poll() is None:
            try:
                self.send("/quit")
                self.proc.wait(timeout=20)
            except (Failure, subprocess.TimeoutExpired, BrokenPipeError):
                self.proc.kill()
                self.proc.wait()
        self.transcript.close()


# --- cursor paths ---------------------------------------------------------------


def path(start: int, target: int) -> str:
    """Keys that move the shared cursor from start to target (w/s rows, a/d columns)."""
    keys = ""
    row, col = divmod(start, 8)
    trow, tcol = divmod(target, 8)
    keys += "w" * max(0, row - trow) + "s" * max(0, trow - row)
    keys += "a" * max(0, col - tcol) + "d" * max(0, tcol - col)
    return keys


# --- checkers: the same rules as apps/checkers/program/main.roc ------------------

CHECKERS_START = ".b.b.b.bb.b.b.b..b.b.b.b................r.r.r.r..r.r.r.rr.r.r.r."


def ck_side(cell: str) -> str:
    return "red" if cell in "rR" else "black" if cell in "bB" else ""


def ck_dirs(piece: str) -> list[int]:
    if piece in "RB":
        return [0, 1, 2, 3]
    return [0, 1] if piece == "r" else [2, 3]


def ck_step(index: int, direction: int):
    row, col = divmod(index, 8)
    if direction == 0 and row > 0 and col > 0:
        return index - 9
    if direction == 1 and row > 0 and col < 7:
        return index - 7
    if direction == 2 and row < 7 and col > 0:
        return index + 7
    if direction == 3 and row < 7 and col < 7:
        return index + 9
    return None


def ck_king_row(piece: str, index: int) -> bool:
    return index // 8 == (0 if ck_side(piece) == "red" else 7)


def ck_segment(board: str, src: int, land: int, piece: str) -> str:
    cells = list(board)
    landed = piece.upper() if piece in "rb" and ck_king_row(piece, land) else piece
    cells[src] = "."
    cells[land] = landed
    if abs(land - src) > 9:
        cells[(src + land) // 2] = "."
    return "".join(cells)


def ck_chains(board: str, index: int, piece: str) -> list[list[int]]:
    chains = []
    for direction in ck_dirs(piece):
        mid = ck_step(index, direction)
        if mid is None or ck_side(board[mid]) in ("", ck_side(piece)):
            continue
        land = ck_step(mid, direction)
        if land is None or board[land] != ".":
            continue
        crowned = piece in "rb" and ck_king_row(piece, land)
        after = ck_segment(board, index, land, piece)
        rest = [] if crowned else ck_chains(after, land, after[land])
        chains.extend([[land, *tail] for tail in rest] or [[land]])
    return chains


def ck_moves(board: str, side: str) -> list[tuple[int, list[int]]]:
    pieces = [i for i, cell in enumerate(board) if ck_side(cell) == side]
    jumps = [(i, chain) for i in pieces for chain in ck_chains(board, i, board[i])]
    if jumps:
        return jumps
    steps = []
    for i in pieces:
        for direction in ck_dirs(board[i]):
            land = ck_step(i, direction)
            if land is not None and board[land] == ".":
                steps.append((i, [land]))
    return steps


def ck_apply(board: str, move: tuple[int, list[int]]) -> str:
    src, landings = move
    for land in landings:
        board = ck_segment(board, src, land, board[src])
        src = land
    return board


def ck_choose(board: str, side: str) -> tuple[int, list[int]]:
    """Red plays to win quickly; black feeds pieces, so games end fast."""
    other = "black" if side == "red" else "red"

    def score(move):
        after = ck_apply(board, move)
        replies = ck_moves(after, other)
        threats = sum(1 for m in replies if abs(m[1][0] - m[0]) > 9)
        if side == "red":
            return (len(move[1]) * (abs(move[1][0] - move[0]) > 9), -threats, -move[0])
        return (threats, -len(replies), move[0])

    return max(ck_moves(board, side), key=score)


# --- chess: a fixed scholar's mate line -----------------------------------------------

CHESS_START = "rnbqkbnrpppppppp................................PPPPPPPPRNBQKBNR"
SCHOLARS_MATE = [(52, 36), (12, 28), (61, 34), (1, 18), (59, 31), (6, 21), (31, 13)]


# --- one game ----------------------------------------------------------------------


def play(host: Host, game: str, homes: dict[str, Path], timeout: float) -> dict:
    first, start = (
        ("red", CHECKERS_START) if game == "checkers" else ("white", CHESS_START)
    )
    ann = Player(host, "ann", homes["ann"], timeout)
    bo = Player(host, "bo", homes["bo"], timeout)
    players = [ann, bo]
    try:
        cursor = 40 if game == "checkers" else 52

        # Seats: the first key takes the first open seat.
        ann.keys("x")
        ann.wait_for(f"ann on {first}", lambda r: r.get(f"{first}_player") == "ann")
        bo.keys("x")
        seated = bo.wait_for("bo on black", lambda r: r.get("black_player") == "bo")
        if seated.get("board") != start or seated.get("turn") != first:
            raise Failure(f"{game}: unexpected opening records {seated}")
        log(f"  seats: {first}=ann black=bo (both sides read the same records)")

        # Out of turn and a watcher: the records must not change.
        bo.keys(" " + path(cursor, 0) + " s ")
        cy = Player(host, "cy", homes["cy"], timeout)
        players.append(cy)
        cy.keys("x " + path(cursor, 0) + " s ")
        time.sleep(1.0)
        for who in (ann, bo, cy):
            after = who.records()
            if (
                after.get("board") != start
                or after.get("plies") != 0
                or after.get("turn") != first
            ):
                raise Failure(
                    f"{game}: out-of-turn or watcher input changed the game: {after}"
                )
        cy.quit()
        players.remove(cy)
        log(
            "  out-of-turn input from black and input from a watcher left the game unchanged"
        )

        # The game.
        board = start
        plies = 0
        side = first
        finished = None
        limit = 300 if game == "checkers" else len(SCHOLARS_MATE)
        while plies < limit:
            mover = ann if side == first else bo
            if game == "checkers":
                src, landings = ck_choose(board, side)
                keys = path(cursor, src) + " "
                at = src
                for land in landings:
                    keys += path(at, land) + " "
                    at = land
                board = ck_apply(board, (src, landings))
            else:
                src, land = SCHOLARS_MATE[plies]
                keys = path(cursor, src) + " " + path(src, land) + " "
                cells = list(board)
                cells[land], cells[src] = cells[src], "."
                board = "".join(cells)
                at = land
            cursor = at
            mover.keys(keys)
            plies += 1
            expected_plies = plies
            got = mover.wait_for(
                f"ply {plies}", lambda r, n=expected_plies: r.get("plies") == n
            )
            other = (bo if mover is ann else ann).records()
            if got != other:
                raise Failure(
                    f"{game}: the two sides disagree after ply {plies}: {got} vs {other}"
                )
            if got.get("board") != board:
                raise Failure(
                    f"{game}: board after ply {plies} is {got.get('board')}, expected {board}"
                )
            if got.get("phase") != "playing":
                finished = got
                break
            side = "black" if side == first else first
            if got.get("turn") != side:
                raise Failure(
                    f"{game}: turn after ply {plies} is {got.get('turn')}, expected {side}"
                )
        if finished is None:
            raise Failure(f"{game}: no result after {plies} plies")
        log(
            f"  game over after {plies} plies: phase={finished.get('phase')} "
            f"{first}={finished.get(first)} black={finished.get('black')}"
        )

        # Restart on the same seats, then a disconnect: bo quits, the
        # seat opens to the AI, and the AI answers ann's first move.
        ann.keys("r")
        ann.wait_for(
            "restart", lambda r: r.get("plies") == 0 and r.get("phase") == "playing"
        )
        bo.quit()
        players.remove(bo)
        ann.wait_for(
            "bo's seat open to the AI", lambda r: r.get("black_player") == "AI"
        )
        cursor = 40 if game == "checkers" else 52
        if game == "checkers":
            src, landings = ck_choose(CHECKERS_START, "red")
            keys = path(cursor, src) + " " + path(src, landings[0]) + " "
        else:
            keys = " " + path(52, 36) + " "
        ann.keys(keys)
        after_leave = ann.wait_for(
            "the AI's reply for black", lambda r: r.get("plies") == 2
        )
        if after_leave.get("turn") != first:
            raise Failure(
                f"{game}: after the AI reply the turn is {after_leave.get('turn')}"
            )
        log("  bo left: black's seat shows AI and the AI answered ann's move")
        return {"game": game, "plies": plies, "phase": finished.get("phase")}
    finally:
        for player in players:
            player.quit()


def run(args) -> int:
    root = Path(args.dir or tempfile.mkdtemp(prefix="board-games-"))
    root.mkdir(parents=True, exist_ok=True)
    id_bin = str(Path(args.id_bin).resolve())  # joins run from other directories
    modules = {"checkers": Path(args.checkers_wasm), "chess": Path(args.chess_wasm)}
    topologies = (
        ["host-join", "central"] if args.topology == "both" else [args.topology]
    )
    games = ["checkers", "chess"] if args.game == "both" else [args.game]
    results = []
    port = args.port_base
    started = time.time()
    try:
        for topology in topologies:
            for game in games:
                log(f"== {topology} / {game}")
                case = root / f"{topology}-{game}"
                host_dir = case / ("ann-host" if topology == "host-join" else "server")
                host = Host(id_bin, host_dir, game, modules[game], port, args.timeout)
                port += 1
                homes = {
                    "ann": host_dir if topology == "host-join" else case / "ann",
                    "bo": case / "bo",
                    "cy": case / "cy",
                }
                try:
                    outcome = play(host, game, homes, args.timeout)
                finally:
                    host.stop()
                results.append({"topology": topology, **outcome})
                log(f"  PASS {topology}/{game}")
    except Failure as error:
        log(f"FAIL: {error}")
        log(
            f"RESULT verdict=FAIL elapsed_s={int(time.time() - started)} passed={len(results)} run_dir={root}"
        )
        return 1
    summary = " ".join(
        f"{r['topology']}/{r['game']}={r['phase']}@{r['plies']}" for r in results
    )
    log(
        f"RESULT verdict=PASS elapsed_s={int(time.time() - started)} games={summary} run_dir={root}"
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--id-bin", required=True)
    parser.add_argument("--checkers-wasm", required=True)
    parser.add_argument("--chess-wasm", required=True)
    parser.add_argument("--dir")
    parser.add_argument("--port-base", type=int, default=47310)
    parser.add_argument("--timeout", type=float, default=60.0)
    parser.add_argument(
        "--topology", choices=["host-join", "central", "both"], default="both"
    )
    parser.add_argument("--game", choices=["checkers", "chess", "both"], default="both")
    return run(parser.parse_args())


if __name__ == "__main__":
    sys.exit(main())
