"""CA bundle resolution.

Proxy handling itself is not reimplemented here: `urllib.request` honours
`HTTPS_PROXY`/`HTTP_PROXY`/`NO_PROXY` natively via `urllib.request.getproxies`
and `ProxyHandler`, unlike Node's `fetch`. Only the CA bundle path needs
explicit resolution, since Python has no single standard env var for it.
"""

from __future__ import annotations


def resolve_ca_file(env: dict[str, str]) -> str | None:
    """Resolves the CA bundle file path to trust, honouring pip's and requests' conventions."""
    return env.get("PIP_CERT") or env.get("REQUESTS_CA_BUNDLE") or env.get("SSL_CERT_FILE")
