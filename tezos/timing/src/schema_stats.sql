
PRAGMA foreign_keys = ON;
PRAGMA synchronous = OFF;

CREATE TABLE IF NOT EXISTS blocks (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  hash TEXT UNIQUE,
  actions_count INTEGER,
  checkout_time_irmin REAL,
  checkout_time_tezedge REAL,
  commit_time_irmin REAL,
  commit_time_tezedge REAL,
  timestamp_secs INTEGER,
  timestamp_nanos INTEGER,
  duration_millis INTEGER
);

CREATE TABLE IF NOT EXISTS operations (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  hash TEXT UNIQUE
);

CREATE TABLE IF NOT EXISTS contexts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  hash TEXT UNIQUE
);

CREATE TABLE IF NOT EXISTS keys (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  key TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS actions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT,
  key_root TEXT,
  key_id INTEGER DEFAULT NULL,
  irmin_time REAL,
  tezedge_time REAL,
  block_id INTEGER DEFAULT NULL,
  operation_id INTEGER DEFAULT NULL,
  context_id INTEGER DEFAULT NULL,
  FOREIGN KEY(key_id) REFERENCES keys(id) DEFERRABLE INITIALLY DEFERRED,
  FOREIGN KEY(block_id) REFERENCES blocks(id) DEFERRABLE INITIALLY DEFERRED,
  FOREIGN KEY(operation_id) REFERENCES operations(id) DEFERRABLE INITIALLY DEFERRED,
  FOREIGN KEY(context_id) REFERENCES contexts(id) DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS block_action_stats (
  id INTEGER PRIMARY KEY AUTOINCREMENT,

  root TEXT NOT NULL,

  tezedge_count INTEGER DEFAULT 0,
  irmin_count INTEGER DEFAULT 0,

  tezedge_mean_time REAL DEFAULT NULL,
  tezedge_max_time REAL DEFAULT NULL,
  tezedge_total_time REAL DEFAULT NULL,

  tezedge_mem_time REAL DEFAULT NULL,
  tezedge_mem_tree_time REAL DEFAULT NULL,
  tezedge_find_time REAL DEFAULT NULL,
  tezedge_find_tree_time REAL DEFAULT NULL,
  tezedge_add_time REAL DEFAULT NULL,
  tezedge_add_tree_time REAL DEFAULT NULL,
  tezedge_remove_time REAL DEFAULT NULL,

  irmin_mean_time REAL DEFAULT NULL,
  irmin_max_time REAL DEFAULT NULL,
  irmin_total_time REAL DEFAULT NULL,

  irmin_mem_time REAL DEFAULT NULL,
  irmin_mem_tree_time REAL DEFAULT NULL,
  irmin_find_time REAL DEFAULT NULL,
  irmin_find_tree_time REAL DEFAULT NULL,
  irmin_add_time REAL DEFAULT NULL,
  irmin_add_tree_time REAL DEFAULT NULL,
  irmin_remove_time REAL DEFAULT NULL,

  block_id INTEGER DEFAULT NULL,
  FOREIGN KEY(block_id) REFERENCES blocks(id) DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE IF NOT EXISTS global_action_stats (
  id INTEGER PRIMARY KEY AUTOINCREMENT,

  root TEXT NOT NULL,
  action_name TEXT NOT NULL,
  context_name TEXT NOT NULL,
  total_time REAL DEFAULT 0.0,
  actions_count INTEGER DEFAULT 0,

  one_to_ten_us_count INTEGER DEFAULT 0,
  one_to_ten_us_mean_time REAL DEFAULT 0.0,
  one_to_ten_us_max_time REAL DEFAULT 0.0,
  one_to_ten_us_total_time REAL DEFAULT 0.0,

  ten_to_one_hundred_us_count INTEGER DEFAULT 0,
  ten_to_one_hundred_us_mean_time REAL DEFAULT 0.0,
  ten_to_one_hundred_us_max_time REAL DEFAULT 0.0,
  ten_to_one_hundred_us_total_time REAL DEFAULT 0.0,

  one_hundred_us_to_one_ms_count INTEGER DEFAULT 0,
  one_hundred_us_to_one_ms_mean_time REAL DEFAULT 0.0,
  one_hundred_us_to_one_ms_max_time REAL DEFAULT 0.0,
  one_hundred_us_to_one_ms_total_time REAL DEFAULT 0.0,

  one_to_ten_ms_count INTEGER DEFAULT 0,
  one_to_ten_ms_mean_time REAL DEFAULT 0.0,
  one_to_ten_ms_max_time REAL DEFAULT 0.0,
  one_to_ten_ms_total_time REAL DEFAULT 0.0,

  ten_to_one_hundred_ms_count INTEGER DEFAULT 0,
  ten_to_one_hundred_ms_mean_time REAL DEFAULT 0.0,
  ten_to_one_hundred_ms_max_time REAL DEFAULT 0.0,
  ten_to_one_hundred_ms_total_time REAL DEFAULT 0.0,

  one_hundred_ms_to_one_s_count INTEGER DEFAULT 0,
  one_hundred_ms_to_one_s_mean_time REAL DEFAULT 0.0,
  one_hundred_ms_to_one_s_max_time REAL DEFAULT 0.0,
  one_hundred_ms_to_one_s_total_time REAL DEFAULT 0.0,

  one_to_ten_s_count INTEGER DEFAULT 0,
  one_to_ten_s_mean_time REAL DEFAULT 0.0,
  one_to_ten_s_max_time REAL DEFAULT 0.0,
  one_to_ten_s_total_time REAL DEFAULT 0.0,

  ten_to_one_hundred_s_count INTEGER DEFAULT 0,
  ten_to_one_hundred_s_mean_time REAL DEFAULT 0.0,
  ten_to_one_hundred_s_max_time REAL DEFAULT 0.0,
  ten_to_one_hundred_s_total_time REAL DEFAULT 0.0,

  one_hundred_s_count INTEGER DEFAULT 0,
  one_hundred_s_mean_time REAL DEFAULT 0.0,
  one_hundred_s_max_time REAL DEFAULT 0.0,
  one_hundred_s_total_time REAL DEFAULT 0.0,

  UNIQUE(root, action_name, context_name)
);
