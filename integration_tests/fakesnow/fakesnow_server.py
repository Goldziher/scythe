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


class SafeJSONResponse(JSONResponse):
    """Serialize Snowflake values such as Decimal and datetime as strings."""

    def render(self, content: Any) -> bytes:
        return json.dumps(content, default=str).encode("utf-8")


def json_value(value: Any, column_type: str) -> Any:
    """Convert fakesnow's Arrow timestamp struct to Snowflake JSON wire format."""
    column_type = column_type.upper()
    if value is None or not column_type.startswith("TIMESTAMP"):
        return value
    if value["epoch"] is None:
        return None

    timestamp = f"{value['epoch']}.{value['fraction']:09d}"
    if column_type == "TIMESTAMP_TZ":
        return f"{timestamp} {value['timezone']}"
    return timestamp


async def node_query_request(request: Request) -> JSONResponse:
    """Add the inline JSON rowset expected by the Node.js driver."""
    request_body = await request.body()
    if request.headers.get("Content-Encoding") == "gzip":
        request_body = gzip.decompress(request_body)
    sql_text = json.loads(request_body)["sqlText"]

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
            [
                json_value(row[column], column_types[index])
                for index, column in enumerate(table.column_names)
            ]
            for row in table.to_pylist()
        ]
    else:
        rows = []

    data.update(
        {
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


routes = [
    Route(route.path, node_query_request, methods=["POST"])
    if isinstance(route, Route) and route.path == "/queries/v1/query-request"
    else route
    for route in server.app.routes
]
app = Starlette(routes=routes)


if __name__ == "__main__":
    uvicorn.run(app, host="0.0.0.0", port=64616)
