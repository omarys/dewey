# 2. SQLite with WAL Mode and Thread-Safe Mutex Connection

We chose embedded SQLite (via `rusqlite` bundled) with Write-Ahead Logging (`PRAGMA journal_mode = WAL`) and an `Arc<Mutex<Connection>>` wrapper for Dewey's data layer. SQLite provides single-file relational storage with foreign key cascade guarantees (`series` -> `chapters` -> `progress`) and instant startup times. Critical sections remain brief while asynchronous background operations (scanning, downloading) batch their writes inside explicit transactions (`batch_record_chapters`) to minimize disk sync overhead.
