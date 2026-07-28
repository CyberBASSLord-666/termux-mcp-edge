#!/usr/bin/env python3
"""Atomically publish one already-validated file without replacing a name."""

from __future__ import annotations

import argparse
import ctypes
import hashlib
import os
import re
import stat
import sys
from typing import NoReturn


def fail(reason: str) -> NoReturn:
    print(f"[verified-file-commit] ERROR: {reason}", file=sys.stderr)
    raise SystemExit(1)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(add_help=False)
    parser.add_argument("--source", required=True)
    parser.add_argument("--destination", required=True)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--mode", required=True)
    try:
        args = parser.parse_args()
    except SystemExit:
        fail("arguments_invalid")
    if (
        not os.path.isabs(args.source)
        or not os.path.isabs(args.destination)
        or os.path.normpath(args.source) != args.source
        or os.path.normpath(args.destination) != args.destination
        or args.source == args.destination
        or re.fullmatch(r"[0-9a-f]{64}", args.sha256) is None
        or re.fullmatch(r"0?[0-7]{3}", args.mode) is None
    ):
        fail("arguments_invalid")
    args.mode_value = int(args.mode, 8)
    return args


def main() -> None:
    args = parse_args()
    source_fd = -1
    destination_parent_fd = -1
    try:
        source_fd = os.open(
            args.source,
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
        )
        source_stat = os.fstat(source_fd)
        if (
            not stat.S_ISREG(source_stat.st_mode)
            or stat.S_IMODE(source_stat.st_mode) != args.mode_value
            or source_stat.st_uid != os.getuid()
            or source_stat.st_size < 1
        ):
            fail("source_invalid")

        digest = hashlib.sha256()
        while True:
            chunk = os.read(source_fd, 1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
        if digest.hexdigest() != args.sha256:
            fail("source_digest_mismatch")

        destination_parent, destination_name = os.path.split(args.destination)
        if not destination_name:
            fail("destination_invalid")
        destination_parent_fd = os.open(
            destination_parent,
            os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
        )
        destination_parent_stat = os.fstat(destination_parent_fd)
        if (
            not stat.S_ISDIR(destination_parent_stat.st_mode)
            or stat.S_IMODE(destination_parent_stat.st_mode) != 0o700
            or destination_parent_stat.st_uid != os.getuid()
        ):
            fail("destination_parent_invalid")

        libc = ctypes.CDLL(None, use_errno=True)
        libc.linkat.argtypes = [
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_int,
        ]
        libc.linkat.restype = ctypes.c_int
        if libc.linkat(
            -100,
            f"/proc/self/fd/{source_fd}".encode(),
            destination_parent_fd,
            os.fsencode(destination_name),
            0x400,
        ) != 0:
            fail("destination_conflict")
    except OSError:
        fail("publication_failed")
    finally:
        if destination_parent_fd >= 0:
            os.close(destination_parent_fd)
        if source_fd >= 0:
            os.close(source_fd)


if __name__ == "__main__":
    main()
