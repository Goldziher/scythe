"""CA bundle resolution.

Proxy handling itself is not reimplemented here: `urllib.request` honours
`HTTPS_PROXY`/`HTTP_PROXY`/`NO_PROXY` natively via `urllib.request.getproxies`
and `ProxyHandler`, unlike Node's `fetch`. Only the CA bundle path needs
explicit resolution, since Python has no single standard env var for it.
"""


def resolve_ca_file(env: dict[str, str]) -> str | None:
    """Resolves the CA bundle file path to trust, honouring pip's and requests' conventions.

    An exported-but-empty variable counts as unset, and yields ``None`` rather
    than the empty string an ``or`` chain would otherwise fall through to.
    """
    for variable in ("PIP_CERT", "REQUESTS_CA_BUNDLE", "SSL_CERT_FILE"):
        value = env.get(variable)
        if value:
            return value
    return None
