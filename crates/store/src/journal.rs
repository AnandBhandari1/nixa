pub const JOURNAL_SQL: &str = "
CREATE TABLE IF NOT EXISTS decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at INTEGER NOT NULL,
    kind TEXT NOT NULL,
    terminal TEXT,
    project TEXT,
    choice TEXT NOT NULL,
    reason TEXT,
    proof TEXT,
    outcome TEXT,
    outcome_at INTEGER,
    corrected INTEGER NOT NULL DEFAULT 0,
    goal TEXT,
    member TEXT,
    confidence TEXT,
    key TEXT,
    result TEXT,
    autonomy TEXT,
    source_space TEXT,
    action_space TEXT,
    item TEXT
);
CREATE INDEX IF NOT EXISTS decisions_at ON decisions (at);
CREATE TABLE IF NOT EXISTS journal_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS changes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    at INTEGER NOT NULL,
    terminal TEXT NOT NULL,
    project TEXT,
    goal TEXT,
    files TEXT NOT NULL,
    count INTEGER NOT NULL,
    outcome TEXT NOT NULL,
    report TEXT
);
CREATE INDEX IF NOT EXISTS changes_at ON changes (at);
";

pub const SCORES_SQL: &str = "
CREATE TABLE IF NOT EXISTS scores (
    key TEXT PRIMARY KEY,
    good INTEGER NOT NULL DEFAULT 0,
    bad INTEGER NOT NULL DEFAULT 0,
    updated_at INTEGER NOT NULL
);
";
