class RevisionCache:
    """In-memory cache. A value is valid only for the exact supplied revision."""

    def __init__(self, capacity=12):
        if capacity < 1:
            raise ValueError("capacity must be positive")
        self.capacity = capacity
        self.entries = {}

    def put(self, key, revision, value):
        if key not in self.entries and len(self.entries) >= self.capacity:
            return False
        self.entries[key] = (revision, value)
        return True

    def get(self, key, revision):
        saved = self.entries.get(key)
        return saved[1] if saved is not None and saved[0] == revision else None
