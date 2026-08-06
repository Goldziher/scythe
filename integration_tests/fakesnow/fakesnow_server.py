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


async def inline_json_query_request(request: Request) -> JSONResponse:
    """Add the inline JSON rowset expected by non-Python Snowflake drivers.

    fakesnow's real query-request handler serves Arrow-encoded results
    (`rowsetBase64`) which only the Python connector decodes natively. Every
    other client driver we test against (Node, Go, JDBC, .NET) speaks the
    plain JSON `rowset` wire format instead, so this decodes the Arrow batch
    once and republishes it as the inline JSON array Snowflake's REST API
    would normally return.
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

    # Drop the Arrow payload and force the JSON result format. Some drivers
    # (notably snowflake-jdbc) trust `queryResultFormat`/`rowsetBase64` over
    # the inline `rowset` we inject and will decode Arrow directly using its
    # own schema if we leave those fields in place, ignoring our rows.
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
    statement_type = {
        "INSERT": 0x3100,
        "UPDATE": 0x3200,
        "DELETE": 0x3300,
        "MERGE": 0x3400,
    }.get(sql_text.lstrip().split(maxsplit=1)[0].upper())
    if statement_type is not None:
        data["statementTypeId"] = statement_type

    payload["code"] = "0"
    return SafeJSONResponse(payload)


async def case_insensitive_login_request(request: Request) -> JSONResponse:
    """Advertise case-insensitive column lookups to the JDBC driver.

    Real Snowflake always includes CLIENT_RESULT_COLUMN_CASE_INSENSITIVE in
    the login response's session parameters, which snowflake-jdbc relies on
    to make `ResultSet.getXxx("column")` match its own unquoted-identifier
    uppercasing convention (e.g. `getInt("id")` against a column labeled
    "ID"). fakesnow's login handler omits it, so the driver falls back to an
    exact-case match and every generated lowercase-name lookup fails with
    "Column not found".
    """
    response = await server.login_request(request)
    payload = json.loads(response.body)
    if response.status_code == 200 and payload.get("success"):
        payload["data"]["parameters"].append({"name": "CLIENT_RESULT_COLUMN_CASE_INSENSITIVE", "value": True})
    return SafeJSONResponse(payload)


def _wrap(route: Route) -> Route:
    if route.path == "/queries/v1/query-request":
        return Route(route.path, inline_json_query_request, methods=["POST"])
    if route.path == "/session/v1/login-request":
        return Route(route.path, case_insensitive_login_request, methods=["POST"])
    return route


routes = [_wrap(route) if isinstance(route, Route) else route for route in server.app.routes]
app = Starlette(routes=routes)


if __name__ == "__main__":
    uvicorn.run(app, host="0.0.0.0", port=64616)
