from __future__ import annotations

import urllib.request

import pytest
from scythe_sql.proxy import resolve_ca_file


def test_resolve_ca_file_returns_none_when_no_variable_is_set() -> None:
    assert resolve_ca_file({}) is None


def test_resolve_ca_file_honours_each_supported_variable() -> None:
    assert resolve_ca_file({"PIP_CERT": "/pip.pem"}) == "/pip.pem"
    assert resolve_ca_file({"REQUESTS_CA_BUNDLE": "/requests.pem"}) == "/requests.pem"
    assert resolve_ca_file({"SSL_CERT_FILE": "/ssl.pem"}) == "/ssl.pem"


def test_resolve_ca_file_prefers_pip_cert_then_requests_then_ssl() -> None:
    env = {"PIP_CERT": "/pip.pem", "REQUESTS_CA_BUNDLE": "/requests.pem", "SSL_CERT_FILE": "/ssl.pem"}
    assert resolve_ca_file(env) == "/pip.pem"
    assert resolve_ca_file({key: value for key, value in env.items() if key != "PIP_CERT"}) == "/requests.pem"
    assert resolve_ca_file({"SSL_CERT_FILE": "/ssl.pem"}) == "/ssl.pem"


def test_resolve_ca_file_skips_variables_set_to_an_empty_string() -> None:
    """An exported-but-empty variable means "unset" to pip and requests alike."""
    assert resolve_ca_file({"PIP_CERT": "", "REQUESTS_CA_BUNDLE": "/requests.pem"}) == "/requests.pem"
    assert resolve_ca_file({"PIP_CERT": "", "REQUESTS_CA_BUNDLE": "", "SSL_CERT_FILE": ""}) is None


def test_urllib_resolves_proxies_from_the_environment(monkeypatch: pytest.MonkeyPatch) -> None:
    """Guards the assumption that lets proxy.py skip proxy handling entirely.

    `_fetch` relies on urllib's default ProxyHandler reading these variables, so
    if that ever stopped being true the wrapper would silently bypass a
    corporate proxy rather than fail loudly.
    """
    monkeypatch.setenv("HTTPS_PROXY", "http://proxy.example:3128")
    monkeypatch.delenv("REQUEST_METHOD", raising=False)

    assert urllib.request.getproxies_environment().get("https") == "http://proxy.example:3128"


def test_urllib_honours_no_proxy_for_matching_hosts(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("NO_PROXY", "github.com")

    assert urllib.request.proxy_bypass_environment("github.com") is True
    assert urllib.request.proxy_bypass_environment("example.org") is False
