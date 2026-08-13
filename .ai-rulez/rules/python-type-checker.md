---
priority: high
---

Python type checking in this repository is **pyrefly**. Do not use, install, configure, or add CI steps for `mypy` — this overrides the `mypy --strict` phrasing inherited from the built-in `python-conventions` rule, which names its type checker only as an example.

Invoke pyrefly directly (`pyrefly check -p strict`), not through `poly lint`. poly lists pyrefly as its Python type-check engine, but as of poly 0.21.0 `poly lint` does not actually reach it — verified against both a clean and a deliberately broken file, where only ruff findings appeared. Re-test before assuming a later poly version changed this.

Use the `strict` preset. Under pyrefly's default `basic` preset a wrong return-type annotation goes unreported, which was confirmed by injecting one into generated output; `-p strict` catches it. A type-check step running under `basic` is close enough to no step at all to be misleading.

The rest of the built-in Python conventions are unchanged: `ruff` for formatting and linting at zero warnings, `bandit` for SAST, `pytest` with `pytest-cov`, `uv` with a committed `uv.lock`, and `pip-audit` for dependency CVEs.
