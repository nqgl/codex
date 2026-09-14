CREATE TABLE thread_monitors (
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    command TEXT NOT NULL,
    cwd TEXT NOT NULL,
    trusted INTEGER NOT NULL CHECK(trusted IN (0, 1)),
    PRIMARY KEY(thread_id, name)
);
