"""Shared base exception for every failure this wrapper raises."""

from __future__ import annotations


class ScytheSqlError(RuntimeError):
    """Base class for all scythe-sql wrapper failures.

    Every message in this hierarchy is already prefixed with ``scythe-sql:`` and
    is written to be printed verbatim, so :mod:`scythe_sql.cli` can catch this
    one type instead of enumerating subclasses -- an enumeration that silently
    fell out of date and double-prefixed the security-relevant checksum errors.

    Subclasses :class:`RuntimeError` so any caller already catching
    ``RuntimeError`` keeps working.
    """
