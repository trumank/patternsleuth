#!/usr/bin/env python3
"""
Tiny REPL client. The interpreter lives inside the game; this just shovels
source lines over a socket and prints what comes back.

Usage:
    python repl.py [host] [port]                 # interactive
    python repl.py [host] [port] -c "CODE"       # run an inline snippet
    python repl.py [host] [port] --script FILE   # run a file
    echo "CODE" | python repl.py [host] [port]   # run source from stdin

Protocol: u32 little-endian length, then UTF-8 bytes, in both directions.
Interactive mode accumulates lines until codeop says they form a complete block,
then sends one frame. The one-shot paths wrap the whole source in a single
exec(compile(...)) frame so multiple top-level statements and an `async def main`
work regardless of codeop's trailing-blank-line quirk.
"""

from __future__ import annotations

import argparse
import ast
import codeop
import re
import socket
import struct
import sys


HOST_DEFAULT = "127.0.0.1"
PORT_DEFAULT = 8765

# A run produces zero or more STDOUT/STDERR frames, then one terminating RESULT
# frame carrying the run's result value (usually empty).
TAG_STDOUT = 0
TAG_RESULT = 1
TAG_STDERR = 2


def send_frame(sock: socket.socket, payload: bytes) -> None:
    sock.sendall(struct.pack("<I", len(payload)) + payload)


def recv_frame(sock: socket.socket) -> bytes:
    raw = recv_exact(sock, 4)
    (length,) = struct.unpack("<I", raw)
    return recv_exact(sock, length)


def recv_exact(sock: socket.socket, n: int) -> bytes:
    out = bytearray()
    while len(out) < n:
        chunk = sock.recv(n - len(out))
        if not chunk:
            raise EOFError("connection closed")
        out.extend(chunk)
    return bytes(out)


def recv_reply(sock: socket.socket) -> str:
    """Read frames until the terminating RESULT, printing STDOUT/STDERR frames
    live to the matching local stream as they arrive. Returns the RESULT text."""
    while True:
        frame = recv_frame(sock)
        tag, text = frame[0], frame[1:].decode("utf-8", errors="replace")
        if tag == TAG_RESULT:
            return text
        stream = sys.stderr if tag == TAG_STDERR else sys.stdout
        stream.write(text)
        stream.flush()


def exchange(sock: socket.socket, source: str) -> str:
    send_frame(sock, source.encode("utf-8"))
    return recv_reply(sock)


def write_reply(reply: str) -> None:
    if not reply:
        return
    sys.stdout.write(reply)
    if not reply.endswith("\n"):
        sys.stdout.write("\n")


_DOTTED_EXPR = re.compile(r"^[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*$")


class _Completer:
    def __init__(self, sock: socket.socket) -> None:
        self.sock = sock
        self.matches: list[str] = []

    def complete(self, text: str, state: int):
        if state == 0:
            self.matches = self._compute(text)
        if 0 <= state < len(self.matches):
            return self.matches[state]
        return None

    def _compute(self, text: str) -> list[str]:
        if "." in text:
            expr, _, prefix = text.rpartition(".")
            # Only probe plain dotted names — don't eval `foo()` or `a[b]` on tab.
            if not _DOTTED_EXPR.match(expr):
                return []
            probe = f"print(repr(dir({expr})))"
            base = expr + "."
        else:
            prefix = text
            probe = "print(repr(sorted(set(list(globals()) + dir(__builtins__)))))"
            base = ""
        try:
            reply = exchange(self.sock, probe).strip()
            names = ast.literal_eval(reply)
        except Exception:
            return []
        return [base + n for n in names if n.startswith(prefix)]


def _install_readline(sock: socket.socket) -> None:
    try:
        import readline
    except ImportError:
        return
    completer = _Completer(sock)
    readline.set_completer(completer.complete)
    # Keep '.' out of the delimiter set so `obj.attr` stays one token.
    readline.set_completer_delims(" \t\n`~!@#$%^&*()=+[{]}\\|;:'\",<>?/")
    readline.parse_and_bind("tab: complete")


def run_interactive(host: str, port: int) -> None:
    with socket.create_connection((host, port)) as sock:
        _install_readline(sock)
        compiler = codeop.CommandCompiler()
        buffer: list[str] = []
        ps1, ps2 = ">>> ", "... "
        while True:
            try:
                line = input(ps2 if buffer else ps1)
            except (EOFError, KeyboardInterrupt):
                print()
                return
            buffer.append(line)
            source = "\n".join(buffer)
            try:
                code = compiler(source, "<repl>", "single")
            except (SyntaxError, ValueError, OverflowError) as e:
                print(f"{type(e).__name__}: {e}")
                buffer.clear()
                continue
            if code is None:
                continue  # need more lines
            buffer.clear()
            write_reply(exchange(sock, source))


def run_source(host: str, port: int, source: str, name: str) -> None:
    """Send one block of source as a single exec(compile(...)) frame and print the
    reply. Shared by the -c, --script, and stdin paths."""
    payload = f"exec(compile({source!r}, {name!r}, 'exec'), globals())"
    with socket.create_connection((host, port)) as sock:
        write_reply(exchange(sock, payload))


def run_script(host: str, port: int, path: str) -> None:
    with open(path, encoding="utf-8") as f:
        source = f.read()
    run_source(host, port, source, path)


def main(argv: list[str]) -> None:
    parser = argparse.ArgumentParser(
        description="REPL client for the in-game Python interpreter",
    )
    parser.add_argument("host", nargs="?", default=HOST_DEFAULT)
    parser.add_argument("port", nargs="?", type=int, default=PORT_DEFAULT)
    source = parser.add_mutually_exclusive_group()
    source.add_argument(
        "-c",
        "--code",
        metavar="CODE",
        help="run CODE inline (like python -c) and exit",
    )
    source.add_argument(
        "--script",
        metavar="FILE",
        help="run the given file and exit",
    )
    args = parser.parse_args(argv)
    # Precedence (like `python`): -c, then --script, then piped stdin, else the REPL.
    if args.code is not None:
        run_source(args.host, args.port, args.code, "<repl -c>")
    elif args.script:
        run_script(args.host, args.port, args.script)
    elif not sys.stdin.isatty():
        stdin_source = sys.stdin.read()
        if stdin_source.strip():
            run_source(args.host, args.port, stdin_source, "<stdin>")
    else:
        run_interactive(args.host, args.port)


if __name__ == "__main__":
    main(sys.argv[1:])
