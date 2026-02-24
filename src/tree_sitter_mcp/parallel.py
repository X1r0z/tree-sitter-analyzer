"""Parallel file processing utilities."""

from __future__ import annotations

import concurrent.futures
import contextlib


def run_parallel(
    files: list[str],
    fn: callable,
    *args: object,
) -> list[dict]:
    """Run fn(file, *args) across files in parallel, returning aggregated results."""
    results: list[dict] = []
    with concurrent.futures.ProcessPoolExecutor() as executor:
        futures = [executor.submit(fn, f, *args) for f in files]
        for future in concurrent.futures.as_completed(futures):
            with contextlib.suppress(Exception):
                results.extend(future.result())
    return results
