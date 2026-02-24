"""Parallel file processing utilities."""

from __future__ import annotations

import concurrent.futures
from collections.abc import Callable


def run_parallel(
    files: list[str],
    fn: Callable,
    *args: object,
) -> list:
    """Run fn(file, *args) across files in parallel, returning aggregated results."""
    results: list = []
    errors: list[str] = []
    with concurrent.futures.ProcessPoolExecutor() as executor:
        futures = {executor.submit(fn, f, *args): f for f in files}
        for future in concurrent.futures.as_completed(futures):
            file_path = futures[future]
            try:
                results.extend(future.result())
            except Exception as exc:
                errors.append(f"{file_path}: {type(exc).__name__}: {exc}")

    if errors:
        details = "; ".join(errors[:3])
        if len(errors) > 3:
            details += f"; ... and {len(errors) - 3} more"
        raise RuntimeError(f"Parallel analysis failed for {len(errors)} file(s): {details}")

    return results
