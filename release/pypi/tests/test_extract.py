import pytest
from scythe_sql.extract import BinaryNotFoundError, find_binary_entry


def test_find_binary_entry_finds_binary_among_license_readme_siblings() -> None:
    assert find_binary_entry(["LICENSE", "README.md", "scythe"], "scythe") == "scythe"


def test_find_binary_entry_matches_by_basename_when_nested() -> None:
    assert (
        find_binary_entry(["scythe-x86_64-unknown-linux-gnu/scythe"], "scythe")
        == "scythe-x86_64-unknown-linux-gnu/scythe"
    )


def test_find_binary_entry_raises_listing_contents_when_absent() -> None:
    with pytest.raises(BinaryNotFoundError, match=r"LICENSE, README\.md"):
        find_binary_entry(["LICENSE", "README.md"], "scythe")


def test_find_binary_entry_distinguishes_scythe_from_scythe_exe() -> None:
    assert find_binary_entry(["scythe.exe"], "scythe.exe") == "scythe.exe"
    with pytest.raises(BinaryNotFoundError):
        find_binary_entry(["scythe.exe"], "scythe")
