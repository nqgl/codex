ALTER TABLE thread_monitors
ADD COLUMN running INTEGER NOT NULL DEFAULT 1 CHECK(running IN (0, 1));
