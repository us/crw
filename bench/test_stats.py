#!/usr/bin/env python3
"""Named test artifact for the paired-delta stats (DoD). The real cases live in
stats._selfcheck (fixed synthetic arrays: width, null, trivial, boundary,
non-degenerate, misalignment, routing, latency-median). Runnable directly or
under pytest."""

from stats import _selfcheck


def test_stats():
    assert _selfcheck() == 0


if __name__ == "__main__":
    test_stats()
    print("test_stats OK")
