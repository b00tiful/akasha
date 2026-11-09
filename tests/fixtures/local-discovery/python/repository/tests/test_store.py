from cache.store import RevisionCache


def test_exact_revision_and_full_replacement():
    cache = RevisionCache(capacity=1)
    assert cache.put("a", 4, "old")
    assert cache.get("a", 3) is None
    assert cache.get("a", 4) == "old"
    assert not cache.put("b", 4, "new key")
    assert cache.put("a", 5, "replacement")
    assert cache.get("a", 5) == "replacement"
