from scythe_sql.preinstalled import has_matching_path_binary


def test_has_matching_path_binary_true_only_on_exact_equality() -> None:
    assert has_matching_path_binary("0.13.0", lambda: "scythe 0.13.0\n") is True


def test_has_matching_path_binary_rejects_newer_path_binary() -> None:
    # A pin must not be silently satisfied by a newer binary already on PATH.
    assert has_matching_path_binary("0.13.0", lambda: "scythe 0.14.0\n") is False


def test_has_matching_path_binary_false_when_not_on_path() -> None:
    def exec_fn() -> str:
        raise FileNotFoundError("scythe not found")

    assert has_matching_path_binary("0.13.0", exec_fn) is False
