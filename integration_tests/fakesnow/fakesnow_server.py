# /// script
# requires-python = ">=3.12"
# dependencies = ["fakesnow[server]==0.11.10"]
# ///
"""Run fakesnow with the JSON result fields required by snowflake-sdk.

This mirrors the Node compatibility fields proposed upstream in
https://github.com/tekumara/fakesnow/pull/313 while keeping CI pinned to a
released fakesnow version.
"""

from __future__ import annotations

import asyncio
import gzip
import importlib
import json
from base64 import b64decode
from decimal import Decimal
from typing import Any

import pyarrow as pa
import uvicorn
from starlette.applications import Starlette
from starlette.requests import Request
from starlette.responses import JSONResponse
from starlette.routing import Route

server = importlib.import_module("fakesnow.server")

# fakesnow shares one DuckDB connection across all requests. Some drivers
# (e.g. gosnowflake) resend a statement on any transient HTTP hiccup, and
# without serialization the resent copy can execute concurrently with the
# still-running original against that shared connection, racing the
# INTEGER PRIMARY KEY auto-increment fakesnow synthesizes and intermittently
# inserting a NULL id. Executing one statement at a time avoids the race.
_QUERY_LOCK = asyncio.Lock()

# Session tokens belonging to clients that decode Snowflake's native Arrow
# result format themselves (currently only snowflake-jdbc). Populated by
# case_insensitive_login_request from the login payload's CLIENT_APP_ID and
# consulted by query_request to decide whether to serve fakesnow's Arrow
# response untouched or convert it to the inline JSON format the other
# drivers (Node, Go, .NET) require. See query_request for why both formats
# are needed.
_ARROW_NATIVE_APP_IDS = {"JDBC"}
_arrow_native_sessions: set[str] = set()

_original_from_binding = server.from_binding


def _patched_from_binding(binding: dict[str, str]) -> Any:
    """Fix fakesnow's FIXED-type binding conversion for non-integer values.

    fakesnow.converter.from_binding() unconditionally does int(value) for
    bindings typed FIXED, but Snowflake's wire protocol uses FIXED for every
    NUMBER parameter regardless of scale -- not just integers. snowflake-jdbc
    sends a bound BigDecimal (e.g. 99.99 for a NUMBER(10, 2) column) as a
    FIXED binding, and int("99.99") raises ValueError, turning the query into
    a 500. Node/Go happen not to exercise this path today because they bind
    decimal params differently, but the bug is in the shared binding
    converter, not the driver. Fall back to Decimal for any FIXED value that
    isn't a bare integer literal.
    """
    if binding.get("type") == "FIXED":
        value = binding["value"]
        try:
            return int(value)
        except ValueError:
            return Decimal(value)
    return _original_from_binding(binding)


server.from_binding = _patched_from_binding


class SafeJSONResponse(JSONResponse):
    """Serialize Snowflake values such as Decimal and datetime as strings."""

    def render(self, content: Any) -> bytes:
        return json.dumps(content, default=str).encode("utf-8")


def json_value(value: Any, column_type: str) -> str | None:
    """Convert an Arrow-decoded cell to the Snowflake JSON wire format.

    Snowflake's real JSON rowset always carries every cell as a string (or
    null) regardless of its logical type, with `rowtype` metadata used by the
    client driver to interpret it. Booleans are encoded as "1"/"0", and Arrow
    timestamp structs are flattened to the `epoch[.fraction][ tz]` string
    format documented by Snowflake.
    """
    column_type = column_type.upper()
    if value is None:
        return None
    if column_type.startswith("TIMESTAMP"):
        if value["epoch"] is None:
            return None
        timestamp = f"{value['epoch']}.{value['fraction']:09d}"
        if column_type == "TIMESTAMP_TZ":
            return f"{timestamp} {value['timezone']}"
        return timestamp
    if isinstance(value, bool):
        return "1" if value else "0"
    return str(value)


async def query_request(request: Request) -> JSONResponse:
    """Serve query results in the format each client driver actually speaks.

    fakesnow's real query-request handler always serves Arrow-encoded
    results (`rowsetBase64`, `queryResultFormat: "arrow"`), which only the
    Python connector and snowflake-jdbc decode natively -- fakesnow has no
    Arrow chunk-download endpoint, but neither driver needs one here since
    the whole (small) result always comes back inline in rowsetBase64 with
    no remote chunks. Every other client driver we test against (Node, Go,
    .NET) speaks the plain JSON `rowset` wire format instead, so for those
    sessions this decodes the Arrow batch once and republishes it as the
    inline JSON array Snowflake's REST API would normally return.

    Which branch a session gets is decided once at login time (see
    case_insensitive_login_request) from the driver's CLIENT_APP_ID, because
    snowflake-jdbc trusts `queryResultFormat`/`rowsetBase64` over the inline
    `rowset` we could inject and would decode Arrow directly using its own
    schema if we left those fields in place while also injecting JSON rows,
    ignoring the injected rows -- so the two formats are mutually exclusive
    per response, not additive.
    """
    request_body = await request.body()
    if request.headers.get("Content-Encoding") == "gzip":
        request_body = gzip.decompress(request_body)
    sql_text = json.loads(request_body)["sqlText"]

    async with _QUERY_LOCK:
        response = await server.query_request(request)
    payload = json.loads(response.body)
    if response.status_code != 200 or not payload.get("success"):
        return response

    data = payload["data"]

    statement_type = {
        "INSERT": 0x3100,
        "UPDATE": 0x3200,
        "DELETE": 0x3300,
        "MERGE": 0x3400,
    }.get(sql_text.lstrip().split(maxsplit=1)[0].upper())
    if statement_type is not None:
        data["statementTypeId"] = statement_type

    payload["code"] = "0"

    token = server.to_token(request)
    if token in _arrow_native_sessions:
        # Arrow-native driver (snowflake-jdbc): serve fakesnow's Arrow
        # response as-is. It already carries Snowflake logical-type
        # metadata per column (see fakesnow.arrow.to_sf_schema), including
        # the epoch/fraction/timezone struct encoding for TIMESTAMP_* that
        # the JDBC driver's ArrowResultChunk parser expects, so TIMESTAMP_NTZ
        # decodes straight into java.time.LocalDateTime without going
        # through the lossy JSON string format below.
        return SafeJSONResponse(payload)

    encoded_rowset = data.get("rowsetBase64")
    if encoded_rowset:
        table = pa.ipc.open_stream(b64decode(encoded_rowset)).read_all()
        column_types = [column["type"] for column in data["rowtype"]]
        rows = [
            [json_value(row[column], column_types[index]) for index, column in enumerate(table.column_names)]
            for row in table.to_pylist()
        ]
    else:
        rows = []

    # Drop the Arrow payload and force the JSON result format for these
    # drivers; see the query_request docstring for why the two formats
    # can't coexist in one response.
    data.pop("rowsetBase64", None)
    data.update(
        {
            "queryResultFormat": "json",
            "chunks": [],
            "returned": len(rows),
            "rowset": rows,
            "version": 1,
        }
    )

    return SafeJSONResponse(payload)


async def login_request(request: Request) -> JSONResponse:
    """Advertise session parameters non-Python Snowflake drivers require.

    Real Snowflake always includes CLIENT_RESULT_COLUMN_CASE_INSENSITIVE and
    CLIENT_PREFETCH_THREADS in the login response's session parameters.
    snowflake-jdbc relies on CLIENT_RESULT_COLUMN_CASE_INSENSITIVE to make
    `ResultSet.getXxx("column")` match its own unquoted-identifier
    uppercasing convention (e.g. `getInt("id")` against a column labeled
    "ID"); without it the driver falls back to an exact-case match and every
    generated lowercase-name lookup fails with "Column not found". Snowflake
    .NET's chunk downloader (SFBlockingChunkDownloaderV3) reads
    CLIENT_PREFETCH_THREADS unconditionally from the session parameter map
    and throws KeyNotFoundException if it's absent, even when the result has
    zero remote chunks. fakesnow's login handler omits both, so this adds
    them for every session.

    This also records, per session token, whether the driver identified
    itself (via CLIENT_APP_ID) as one that decodes Snowflake's native Arrow
    result format -- see query_request.
    """
    body = await request.body()
    if request.headers.get("Content-Encoding") == "gzip":
        body = gzip.decompress(body)
    client_app_id = json.loads(body).get("data", {}).get("CLIENT_APP_ID")

    response = await server.login_request(request)
    payload = json.loads(response.body)
    if response.status_code == 200 and payload.get("success"):
        payload["data"]["parameters"].append({"name": "CLIENT_RESULT_COLUMN_CASE_INSENSITIVE", "value": True})
        payload["data"]["parameters"].append({"name": "CLIENT_PREFETCH_THREADS", "value": 4})
        if client_app_id in _ARROW_NATIVE_APP_IDS:
            _arrow_native_sessions.add(payload["data"]["token"])
    return SafeJSONResponse(payload)


def _wrap(route: Route) -> Route:
    if route.path == "/queries/v1/query-request":
        return Route(route.path, query_request, methods=["POST"])
    if route.path == "/session/v1/login-request":
        return Route(route.path, login_request, methods=["POST"])
    return route


routes = [_wrap(route) if isinstance(route, Route) else route for route in server.app.routes]
app = Starlette(routes=routes)


if __name__ == "__main__":
    uvicorn.run(app, host="0.0.0.0", port=64616)
